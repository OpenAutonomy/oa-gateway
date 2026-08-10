use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::{mpsc, RwLock};

use crate::{AdapterId, Envelope, RouteKey, SubId};

/// Capacity of each subscriber delivery channel.
pub const DEFAULT_CHANNEL_CAPACITY: usize = 64;

/// Composite key for a subscription owned by one adapter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SubscriberKey {
    pub adapter_id: AdapterId,
    pub sub_id: SubId,
}

/// Delivery wrapper so adapters know which of their subscriptions matched.
#[derive(Debug, Clone)]
pub struct Delivery {
    pub sub_id: SubId,
    pub envelope: Envelope,
}

/// Snapshot of engine counters.
#[derive(Debug, Default)]
pub struct EngineStats {
    pub published: AtomicU64,
    pub delivered: AtomicU64,
    pub dropped: AtomicU64,
}

impl EngineStats {
    #[must_use]
    pub fn published(&self) -> u64 {
        self.published.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn delivered(&self) -> u64 {
        self.delivered.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Result of a publish fan-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishOutcome {
    pub matched: usize,
    pub delivered: usize,
    pub dropped: usize,
}

/// Errors from subscribe / unsubscribe.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("duplicate subscription {0}")]
    DuplicateSub(SubId),
    #[error("unknown subscription {0}")]
    UnknownSub(SubId),
}

struct Subscription {
    route: RouteKey,
    tx: mpsc::Sender<Delivery>,
}

#[derive(Default)]
struct State {
    by_key: HashMap<SubscriberKey, Subscription>,
    by_adapter: HashMap<AdapterId, HashSet<SubId>>,
    /// Exact (topic, type_hint) → subscribers.
    exact: HashMap<String, HashMap<String, HashSet<SubscriberKey>>>,
    /// Topic → wildcard subscribers (type_hint is None).
    wildcards: HashMap<String, HashSet<SubscriberKey>>,
}

/// Shared, protocol-neutral pub/sub router.
pub struct Engine {
    state: RwLock<State>,
    stats: EngineStats,
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: RwLock::new(State::default()),
            stats: EngineStats::default(),
        }
    }

    #[must_use]
    pub fn stats(&self) -> &EngineStats {
        &self.stats
    }

    /// Register a subscriber. The adapter owns `tx` and reads [`Delivery`]s.
    pub async fn subscribe(
        &self,
        adapter_id: impl Into<AdapterId>,
        sub_id: impl Into<SubId>,
        route: RouteKey,
        tx: mpsc::Sender<Delivery>,
    ) -> Result<SubscriberKey, EngineError> {
        let key = SubscriberKey {
            adapter_id: adapter_id.into(),
            sub_id: sub_id.into(),
        };
        let mut state = self.state.write().await;
        if state.by_key.contains_key(&key) {
            return Err(EngineError::DuplicateSub(key.sub_id));
        }

        insert_index(&mut state, &key, &route);
        state
            .by_adapter
            .entry(key.adapter_id.clone())
            .or_default()
            .insert(key.sub_id.clone());
        state.by_key.insert(key.clone(), Subscription { route, tx });
        Ok(key)
    }

    pub async fn unsubscribe(
        &self,
        adapter_id: impl Into<AdapterId>,
        sub_id: impl Into<SubId>,
    ) -> Result<(), EngineError> {
        let key = SubscriberKey {
            adapter_id: adapter_id.into(),
            sub_id: sub_id.into(),
        };
        let mut state = self.state.write().await;
        remove_subscription(&mut state, &key).map_err(|()| EngineError::UnknownSub(key.sub_id))
    }

    /// Remove every subscription owned by `adapter_id`.
    pub async fn drop_adapter(&self, adapter_id: impl Into<AdapterId>) -> usize {
        let adapter_id = adapter_id.into();
        let mut state = self.state.write().await;
        let Some(sids) = state.by_adapter.remove(&adapter_id) else {
            return 0;
        };
        let mut removed = 0;
        for sid in sids {
            let key = SubscriberKey {
                adapter_id: adapter_id.clone(),
                sub_id: sid,
            };
            if remove_subscription(&mut state, &key).is_ok() {
                removed += 1;
            }
        }
        removed
    }

    /// Fan out `envelope` to matching subscribers. Never inspects the payload.
    pub async fn publish(&self, envelope: Envelope) -> PublishOutcome {
        self.stats.published.fetch_add(1, Ordering::Relaxed);

        let targets = {
            let state = self.state.read().await;
            collect_targets(&state, &envelope.route)
        };

        let matched = targets.len();
        let mut delivered = 0;
        let mut dropped = 0;
        for (tx, sub_id) in targets {
            let delivery = Delivery {
                sub_id,
                envelope: envelope.clone(),
            };
            match tx.try_send(delivery) {
                Ok(()) => delivered += 1,
                Err(TrySendError::Full(_) | TrySendError::Closed(_)) => dropped += 1,
            }
        }

        self.stats
            .delivered
            .fetch_add(delivered as u64, Ordering::Relaxed);
        self.stats
            .dropped
            .fetch_add(dropped as u64, Ordering::Relaxed);

        PublishOutcome {
            matched,
            delivered,
            dropped,
        }
    }

    #[must_use]
    pub async fn subscription_count(&self) -> usize {
        self.state.read().await.by_key.len()
    }
}

fn insert_index(state: &mut State, key: &SubscriberKey, route: &RouteKey) {
    if let Some(hint) = &route.type_hint {
        state
            .exact
            .entry(route.topic.clone())
            .or_default()
            .entry(hint.clone())
            .or_default()
            .insert(key.clone());
    } else {
        state
            .wildcards
            .entry(route.topic.clone())
            .or_default()
            .insert(key.clone());
    }
}

fn remove_subscription(state: &mut State, key: &SubscriberKey) -> Result<(), ()> {
    let Some(sub) = state.by_key.remove(key) else {
        return Err(());
    };

    if let Some(sids) = state.by_adapter.get_mut(&key.adapter_id) {
        sids.remove(&key.sub_id);
        if sids.is_empty() {
            state.by_adapter.remove(&key.adapter_id);
        }
    }

    if let Some(hint) = &sub.route.type_hint {
        if let Some(by_hint) = state.exact.get_mut(&sub.route.topic) {
            if let Some(set) = by_hint.get_mut(hint) {
                set.remove(key);
                if set.is_empty() {
                    by_hint.remove(hint);
                }
            }
            if by_hint.is_empty() {
                state.exact.remove(&sub.route.topic);
            }
        }
    } else if let Some(set) = state.wildcards.get_mut(&sub.route.topic) {
        set.remove(key);
        if set.is_empty() {
            state.wildcards.remove(&sub.route.topic);
        }
    }

    Ok(())
}

fn collect_targets(state: &State, route: &RouteKey) -> Vec<(mpsc::Sender<Delivery>, SubId)> {
    let mut keys: HashSet<SubscriberKey> = HashSet::new();

    if let Some(set) = state.wildcards.get(&route.topic) {
        keys.extend(set.iter().cloned());
    }
    if let Some(hint) = &route.type_hint {
        if let Some(by_hint) = state.exact.get(&route.topic) {
            if let Some(set) = by_hint.get(hint) {
                keys.extend(set.iter().cloned());
            }
        }
    }

    let mut out = Vec::with_capacity(keys.len());
    for key in keys {
        if let Some(sub) = state.by_key.get(&key) {
            out.push((sub.tx.clone(), key.sub_id));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    use super::*;

    fn ping(topic: &str) -> Envelope {
        Envelope::new(RouteKey::typed(topic, "Ping"), Bytes::from_static(b"ping"))
    }

    fn pong(topic: &str) -> Envelope {
        Envelope::new(RouteKey::typed(topic, "Pong"), Bytes::from_static(b"pong"))
    }

    async fn recv(rx: &mut mpsc::Receiver<Delivery>) -> Envelope {
        timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("timed out waiting for delivery")
            .expect("channel closed")
            .envelope
    }

    async fn expect_none(rx: &mut mpsc::Receiver<Delivery>) {
        match timeout(Duration::from_millis(50), rx.recv()).await {
            Err(_) | Ok(None) => {}
            Ok(Some(delivery)) => panic!("unexpected delivery {:?}", delivery.envelope.route),
        }
    }

    #[tokio::test]
    async fn typed_subscribe_receives_matching_publish() {
        let engine = Engine::new();
        let (tx, mut rx) = mpsc::channel(8);
        engine
            .subscribe("a", "s1", RouteKey::typed("demo", "Ping"), tx)
            .await
            .unwrap();

        let sent = ping("demo").with_header("k", "v");
        let outcome = engine.publish(sent.clone()).await;
        assert_eq!(outcome.delivered, 1);

        let got = recv(&mut rx).await;
        assert_eq!(got.payload, sent.payload);
        assert_eq!(got.headers, sent.headers);
        assert_eq!(got.route, sent.route);
    }

    #[tokio::test]
    async fn typed_subscribe_ignores_other_type() {
        let engine = Engine::new();
        let (tx, mut rx) = mpsc::channel(8);
        engine
            .subscribe("a", "s1", RouteKey::typed("demo", "Ping"), tx)
            .await
            .unwrap();

        engine.publish(pong("demo")).await;
        expect_none(&mut rx).await;
    }

    #[tokio::test]
    async fn wildcard_subscribe_receives_all_types() {
        let engine = Engine::new();
        let (tx, mut rx) = mpsc::channel(8);
        engine
            .subscribe("a", "wild", RouteKey::topic("demo"), tx)
            .await
            .unwrap();

        engine.publish(ping("demo")).await;
        engine.publish(pong("demo")).await;

        let first = recv(&mut rx).await;
        let second = recv(&mut rx).await;
        let mut payloads = vec![
            first.payload.as_ref().to_vec(),
            second.payload.as_ref().to_vec(),
        ];
        payloads.sort();
        assert_eq!(payloads, [b"ping".to_vec(), b"pong".to_vec()]);
    }

    #[tokio::test]
    async fn drop_adapter_unsubscribes_all_routes() {
        let engine = Engine::new();
        let (tx, mut rx) = mpsc::channel(8);
        engine
            .subscribe("a", "s1", RouteKey::typed("demo", "Ping"), tx)
            .await
            .unwrap();
        assert_eq!(engine.subscription_count().await, 1);

        let removed = engine.drop_adapter("a").await;
        assert_eq!(removed, 1);
        assert_eq!(engine.subscription_count().await, 0);

        engine.publish(ping("demo")).await;
        expect_none(&mut rx).await;
    }

    #[tokio::test]
    async fn duplicate_sub_is_rejected() {
        let engine = Engine::new();
        let (tx, _rx) = mpsc::channel(8);
        engine
            .subscribe("a", "s1", RouteKey::topic("demo"), tx.clone())
            .await
            .unwrap();
        let err = engine
            .subscribe("a", "s1", RouteKey::topic("other"), tx)
            .await
            .unwrap_err();
        assert!(matches!(err, EngineError::DuplicateSub(_)));
    }

    #[tokio::test]
    async fn unsubscribe_stops_delivery() {
        let engine = Engine::new();
        let (tx, mut rx) = mpsc::channel(8);
        engine
            .subscribe("a", "s1", RouteKey::typed("demo", "Ping"), tx)
            .await
            .unwrap();
        engine.unsubscribe("a", "s1").await.unwrap();
        engine.publish(ping("demo")).await;
        expect_none(&mut rx).await;
    }

    #[tokio::test]
    async fn backpressure_drops_and_counts() {
        let engine = Engine::new();
        let (tx, _rx) = mpsc::channel(1);
        engine
            .subscribe("a", "s1", RouteKey::typed("demo", "Ping"), tx)
            .await
            .unwrap();

        let first = engine.publish(ping("demo")).await;
        assert_eq!(first.delivered, 1);
        let second = engine.publish(ping("demo")).await;
        assert_eq!(second.dropped, 1);
        assert_eq!(engine.stats().dropped(), 1);
    }
}

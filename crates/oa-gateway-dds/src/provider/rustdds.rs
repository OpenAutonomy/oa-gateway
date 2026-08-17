//! rustdds implementation of [`DdsProvider`].

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration as StdDuration;

use rustdds::no_key::{DataReader, DataWriter};
use rustdds::serialization::{CDRDeserializerAdapter, CDRSerializerAdapter};
use rustdds::{
    policy, DomainParticipant, QosPolicies, QosPolicyBuilder, RTPSEntity, ReadCondition, TopicKind,
};

use crate::qos_xml::{self, Durability, History, QosSpec, Reliability};
use crate::types::{DdsSample, MaDataPayload, TYPE_NAME};
use crate::{DdsError, DdsProvider, DdsSession};

/// First provider. QoS XML is parsed by [`crate::load_qos`].
pub struct RustddsProvider;

impl DdsProvider for RustddsProvider {
    fn name(&self) -> &'static str {
        "rustdds"
    }

    fn join(&self, domain_id: u16, qos_path: &Path) -> Result<Box<dyn DdsSession>, DdsError> {
        let spec = qos_xml::load(qos_path).map_err(DdsError::msg)?;
        let qos = to_rustdds_qos(&spec);
        let participant = DomainParticipant::new(domain_id)
            .map_err(|err| DdsError::msg(format!("join domain {domain_id}: {err}")))?;
        let publisher = participant
            .create_publisher(&qos)
            .map_err(|err| DdsError::msg(format!("create publisher: {err}")))?;
        let subscriber = participant
            .create_subscriber(&qos)
            .map_err(|err| DdsError::msg(format!("create subscriber: {err}")))?;
        Ok(Box::new(RustddsSession {
            participant,
            publisher,
            subscriber,
            qos,
            writers: HashMap::new(),
            readers: HashMap::new(),
        }))
    }
}

struct RustddsSession {
    participant: DomainParticipant,
    publisher: rustdds::Publisher,
    subscriber: rustdds::Subscriber,
    qos: QosPolicies,
    writers: HashMap<String, DataWriter<MaDataPayload, CDRSerializerAdapter<MaDataPayload>>>,
    readers: HashMap<String, DataReader<MaDataPayload, CDRDeserializerAdapter<MaDataPayload>>>,
}

impl DdsSession for RustddsSession {
    fn create_topic(&mut self, name: &str) -> Result<(), DdsError> {
        if self.writers.contains_key(name) {
            return Ok(());
        }
        let topic = self
            .participant
            .create_topic(
                name.to_owned(),
                TYPE_NAME.to_owned(),
                &self.qos,
                TopicKind::NoKey,
            )
            .map_err(|err| DdsError::msg(format!("topic {name}: {err}")))?;
        let writer = self
            .publisher
            .create_datawriter_no_key(&topic, None)
            .map_err(|err| DdsError::msg(format!("writer {name}: {err}")))?;
        let reader = self
            .subscriber
            .create_datareader_no_key(&topic, None)
            .map_err(|err| DdsError::msg(format!("reader {name}: {err}")))?;
        self.writers.insert(name.to_owned(), writer);
        self.readers.insert(name.to_owned(), reader);
        Ok(())
    }

    fn write(&self, topic: &str, sample: DdsSample) -> Result<(), DdsError> {
        let writer = self
            .writers
            .get(topic)
            .ok_or_else(|| DdsError::msg(format!("no writer for {topic}")))?;
        writer
            .write(MaDataPayload::from_sample(&sample), None)
            .map_err(|err| DdsError::msg(format!("write {topic}: {err}")))
    }

    fn poll_inbound(&mut self) -> Result<Vec<(String, DdsSample)>, DdsError> {
        // SampleInfo.writer_guid is the DataWriter, not the participant.
        // Every endpoint on this participant shares the prefix.
        let mine = self.participant.guid_prefix();
        let mut out = Vec::new();
        for (topic, reader) in &mut self.readers {
            let samples = reader
                .take(32, ReadCondition::any())
                .map_err(|err| DdsError::msg(format!("take {topic}: {err}")))?;
            for data in samples {
                if data.sample_info().writer_guid().prefix == mine {
                    continue;
                }
                let sample = data
                    .into_value()
                    .into_sample()
                    .map_err(|err| DdsError::msg(format!("{topic}: {err}")))?;
                out.push((topic.clone(), sample));
            }
        }
        Ok(out)
    }
}

fn to_rustdds_qos(spec: &QosSpec) -> QosPolicies {
    let mut b = QosPolicyBuilder::new();
    b = match spec.reliability {
        Reliability::Reliable => b.reliability(policy::Reliability::Reliable {
            max_blocking_time: rustdds::Duration::from_std(StdDuration::from_secs(1)),
        }),
        Reliability::BestEffort => b.reliability(policy::Reliability::BestEffort),
    };
    b = match spec.durability {
        Durability::Volatile => b.durability(policy::Durability::Volatile),
        Durability::TransientLocal => b.durability(policy::Durability::TransientLocal),
    };
    b = match spec.history {
        History::KeepLast { depth } => b.history(policy::History::KeepLast { depth }),
        History::KeepAll => b.history(policy::History::KeepAll),
    };
    b.build()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use bytes::Bytes;
    use oa_gateway_agra::{WrapperKind, WrapperMeta};

    use super::*;
    use crate::types::DdsSample;

    fn ping() -> DdsSample {
        DdsSample {
            meta: WrapperMeta {
                kind: WrapperKind::Rx,
                message_type_enum: "PING".into(),
                originator_uuid: None,
                rx_payload_id: None,
                command_id: None,
                destination_routing: None,
            },
            encoded: Bytes::from_static(br#"{"Ping":{"n":1}}"#),
        }
    }

    fn qos_path() -> &'static Path {
        Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../config/dds-qos.xml"
        ))
    }

    #[test]
    fn two_participants_exchange_and_skip_local() {
        let provider = RustddsProvider;
        let mut a = provider.join(91, qos_path()).unwrap();
        let mut b = provider.join(91, qos_path()).unwrap();
        a.create_topic("demo").unwrap();
        b.create_topic("demo").unwrap();

        let deadline = Instant::now() + Duration::from_secs(8);
        let mut got = None;
        while Instant::now() < deadline {
            a.write("demo", ping()).unwrap();
            if let Some((_, sample)) = b.poll_inbound().unwrap().into_iter().next() {
                got = Some(sample);
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let got = got.expect("participant b should see a write from a");
        assert_eq!(got.encoded.as_ref(), br#"{"Ping":{"n":1}}"#);
        assert!(
            a.poll_inbound().unwrap().is_empty(),
            "local writes must not appear as inbound"
        );
    }
}

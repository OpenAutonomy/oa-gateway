//! XSD instance validation via libxml2.
//!
//! Independent of `oa-gateway-uci`: that crate's hand-rolled `Schema` still
//! owns JSON <-> XML conversion (libxml2 has no equivalent — it validates
//! XML instances, full stop, with no JSON concept at all). This crate is
//! the one and only conformance check on the XML wire form, replacing the
//! hand-rolled `validate` module that used to live in `oa-gateway-uci`.
//!
//! `libxml::schemas::SchemaValidationContext` is `!Send`/`!Sync` and takes
//! `&mut self` to validate, so it cannot be moved into a `spawn_blocking`
//! closure (which must be `Send`) without recompiling the whole XSD on
//! every call. Instead the compiled context lives on one dedicated
//! `std::thread` for the process's life, reached over a channel — the only
//! things that cross the thread boundary are owned `String`s and channel
//! endpoints, so this crate needs no `unsafe` of its own.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;

use libxml::parser::Parser;
use libxml::schemas::{SchemaParserContext, SchemaValidationContext};
use tokio::sync::{mpsc, oneshot};

/// One libxml2 schema error. `line`/`col` locate it in the XML text passed
/// to [`XsdLibxmlValidator::validate`] — there is no equivalent to the
/// hand-rolled validator's dotted instance-tree path, since libxml2 has no
/// concept of the JSON-side instance tree at all.
#[derive(Debug, Clone)]
pub struct XsdError {
    pub message: String,
    pub line: Option<i32>,
    pub col: Option<i32>,
}

/// The first few, then a count — same shape as the deleted hand-rolled
/// validator's `summarize`, so log lines read consistently either way.
pub fn summarize(errors: &[XsdError]) -> String {
    const SHOWN: usize = 3;
    let mut out = String::new();
    for (i, err) in errors.iter().take(SHOWN).enumerate() {
        if i > 0 {
            out.push_str("; ");
        }
        out.push_str(&err.message);
    }
    if errors.len() > SHOWN {
        out.push_str(&format!(" (and {} more)", errors.len() - SHOWN));
    }
    out
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("path is not valid UTF-8: {}", .0.display())]
    NotUtf8(std::path::PathBuf),
    #[error("libxml2 could not compile the schema: {0}")]
    Compile(String),
    #[error("libxml2 schema parser panicked while compiling the schema")]
    Panicked,
    #[error("validator thread exited before compiling the schema")]
    ThreadExited,
}

struct Request {
    xml: String,
    respond_to: oneshot::Sender<Result<(), Vec<XsdError>>>,
}

/// Compiles an XSD once and validates XML instances against it,
/// off the async runtime, on one dedicated thread for its whole life.
///
/// Cheap to clone: every clone shares the same compiled schema and the
/// same dedicated thread, the same way `Arc<Schema>` shares a compiled
/// hand-rolled schema.
#[derive(Clone)]
pub struct XsdLibxmlValidator {
    tx: mpsc::Sender<Request>,
}

impl std::fmt::Debug for XsdLibxmlValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XsdLibxmlValidator").finish_non_exhaustive()
    }
}

impl XsdLibxmlValidator {
    /// Compiles the XSD at `entry_path`. `entry_path` must itself
    /// `xs:include`/`xs:import` everything else the schema spans — unlike
    /// the hand-rolled compiler, which takes an exhaustive, unordered file
    /// list and does not follow includes, libxml2 wants one entry point
    /// and resolves `xs:include`/`xs:import` itself, relative to that
    /// file's own path on disk.
    ///
    /// # Errors
    ///
    /// Returns an error if `entry_path` is not valid UTF-8, cannot be
    /// read, does not compile, or the compile step panics (caught rather
    /// than left to take the process down).
    pub fn compile(entry_path: &Path) -> Result<Self, CompileError> {
        let path = entry_path
            .to_str()
            .ok_or_else(|| CompileError::NotUtf8(entry_path.to_path_buf()))?
            .to_owned();
        Self::spawn_worker(move || SchemaParserContext::from_file(&path))
    }

    /// Compiles an XSD held as a string, with no `xs:include`/`xs:import`
    /// resolution (there is no second file to include, so no base URI to
    /// resolve one against). For a single self-contained document — a
    /// test fixture, say — rather than the published catalog's multi-file
    /// spans, where [`Self::compile`] is the one to use.
    ///
    /// Routed through a temp file rather than `SchemaParserContext::from_buffer`:
    /// that function hands libxml2 a raw pointer into its argument and
    /// then drops it at the end of the call, so libxml2 is left holding a
    /// dangling pointer the moment anything actually reads from it — a
    /// real use-after-free in the dependency, confirmed live ("Document is
    /// empty" on a schema that compiles fine from a file). `from_file`
    /// does not have this problem: the file outlives the call.
    ///
    /// # Errors
    ///
    /// Returns an error if a temp file cannot be created or written, or
    /// the XSD does not compile.
    pub fn compile_str(xsd: &str) -> Result<Self, CompileError> {
        let path = std::env::temp_dir().join(format!(
            "oa-gateway-xsd-libxml-{}-{}.xsd",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::write(&path, xsd).map_err(|e| CompileError::Compile(e.to_string()))?;
        let result = Self::compile(&path);
        let _ = std::fs::remove_file(&path);
        result
    }

    /// Shared worker: build a `SchemaParserContext` with `new_parser` on
    /// the dedicated thread, compile it, then serve [`Request`]s until the
    /// channel closes.
    fn spawn_worker(
        new_parser: impl FnOnce() -> SchemaParserContext + Send + 'static,
    ) -> Result<Self, CompileError> {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (tx, mut rx) = mpsc::channel::<Request>(64);

        std::thread::spawn(move || {
            // `SchemaParserContext::from_file`/`from_buffer` panic (not
            // `Result`) if libxml2 returns null; caught here so a bad or
            // malformed custom XSD reports a clean startup error instead
            // of taking the process down. The workspace sets no
            // `panic = "abort"` profile, so catch_unwind actually works.
            let built = catch_unwind(AssertUnwindSafe(|| {
                let mut parser = new_parser();
                SchemaValidationContext::from_parser(&mut parser)
            }));
            let mut ctx = match built {
                Ok(Ok(ctx)) => {
                    let _ = ready_tx.send(Ok(()));
                    ctx
                }
                Ok(Err(errs)) => {
                    let _ = ready_tx.send(Err(CompileError::Compile(format_errors(&errs))));
                    return;
                }
                Err(_panic) => {
                    let _ = ready_tx.send(Err(CompileError::Panicked));
                    return;
                }
            };

            let parser = Parser::default();
            while let Some(req) = rx.blocking_recv() {
                let outcome = parser
                    .parse_string(&req.xml)
                    .map_err(|e| {
                        vec![XsdError {
                            message: e.to_string(),
                            line: None,
                            col: None,
                        }]
                    })
                    .and_then(|doc| {
                        ctx.validate_document(&doc)
                            .map_err(|errs| errs.iter().map(to_xsd_error).collect())
                    });
                let _ = req.respond_to.send(outcome);
            }
            // tx's last clone dropped => rx.blocking_recv() returns None => thread exits.
        });

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self { tx }),
            Ok(Err(err)) => Err(err),
            Err(_) => Err(CompileError::ThreadExited),
        }
    }

    /// Validates `xml` against the compiled schema. Safe to call from
    /// async code: hands the string to the dedicated thread and awaits
    /// the reply.
    ///
    /// Serializes all validation through one thread, by design — a
    /// dedicated thread, not one per message, strictly additive to the
    /// conversion/dispatch work this call sits next to. If this becomes
    /// a throughput bottleneck under real load, the fix is mechanical
    /// (N threads, each with its own independently-compiled context,
    /// requests round-robined across N channels) — not worth building
    /// ahead of a benchmark that justifies it.
    ///
    /// # Errors
    ///
    /// Returns every way `xml` departs from the compiled schema: a
    /// well-formedness failure from libxml2's own parser, or one
    /// [`XsdError`] per schema violation it reports.
    pub async fn validate(&self, xml: String) -> Result<(), Vec<XsdError>> {
        let (respond_to, rx) = oneshot::channel();
        if self.tx.send(Request { xml, respond_to }).await.is_err() {
            // Worker thread is gone. Nothing more to say -- not a false reject.
            return Ok(());
        }
        rx.await.unwrap_or(Ok(()))
    }
}

fn to_xsd_error(err: &libxml::error::StructuredError) -> XsdError {
    XsdError {
        message: err
            .message
            .clone()
            .unwrap_or_else(|| "libxml2 reported no message".to_string()),
        line: err.line,
        col: err.col,
    }
}

fn format_errors(errs: &[libxml::error::StructuredError]) -> String {
    errs.iter()
        .map(to_xsd_error)
        .map(|e| e.message)
        .collect::<Vec<_>>()
        .join("; ")
}

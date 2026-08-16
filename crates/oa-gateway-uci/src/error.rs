/// Why a payload could not be converted, or a schema could not be compiled.
///
/// Conversion is forgiving about undeclared fields; those are not
/// errors here. [`Self::TooDeep`] is the stack-safety bound, not a
/// schema constraint. [`Self::Xsd`] is compile-time only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UciError {
    #[error("not valid UTF-8")]
    NotUtf8,
    #[error("invalid OMS JSON: {0}")]
    Json(String),
    #[error("invalid UCI XML: {0}")]
    Xml(String),
    #[error("unknown global element '{0}'")]
    UnknownElement(String),
    #[error("unknown type '{0}'")]
    UnknownType(String),
    #[error("unsupported or malformed XSD: {0}")]
    Xsd(String),
    #[error("at {path}: {message}")]
    At { path: String, message: String },
    #[error("at {path}: nesting is deeper than {} elements", crate::MAX_DEPTH)]
    TooDeep { path: String },
}

impl UciError {
    pub(crate) fn at(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::At {
            path: path.into(),
            message: message.into(),
        }
    }

    pub(crate) fn too_deep(path: impl Into<String>) -> Self {
        Self::TooDeep { path: path.into() }
    }
}

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "ASFILED_SEC_USER_AGENT is not set. SEC access policy requires a declared \
         User-Agent with a real contact address, e.g.\n\n    \
         export ASFILED_SEC_USER_AGENT=\"Jane Doe jane@example.com\"\n"
    )]
    MissingUserAgent,

    #[error("404 from EDGAR: {0}")]
    NotFound(String),

    #[error("exhausted {attempts} attempts for {url}")]
    Exhausted { url: String, attempts: u32 },

    #[error("unexpected status {status} for {url}")]
    Status { url: String, status: u16 },

    #[error("http error for {url}: {source}")]
    Http {
        url: String,
        #[source]
        source: Box<ureq::Error>,
    },

    #[error("io error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid json from {url}: {source}")]
    Json {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("database error: {0}")]
    Db(#[from] duckdb::Error),

    #[error("invalid concept map: {0}")]
    ConceptMap(String),
}

//! Rate-limited, disk-cached HTTP client for SEC EDGAR.
//!
//! Every SEC request in the project goes through here. Two reasons: the
//! 10 req/s fair-access ceiling has to be enforced in one place to mean
//! anything, and the on-disk cache is what makes a full rebuild possible with
//! no network — which in turn is what makes historical scoring reproducible.

use std::io::Read;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::{Config, SEC_RATE_LIMIT_PER_SEC};
use crate::error::Error;
use crate::Result;

const RETRY_STATUSES: [u16; 5] = [429, 500, 502, 503, 504];
const MAX_ATTEMPTS: u32 = 5;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Token bucket shared by every thread that touches SEC hosts.
struct RateLimiter {
    rate: f64,
    capacity: f64,
    state: Mutex<(f64, Instant)>, // (tokens, last update)
}

impl RateLimiter {
    fn new(rate_per_sec: f64) -> Self {
        Self {
            rate: rate_per_sec,
            capacity: rate_per_sec,
            state: Mutex::new((rate_per_sec, Instant::now())),
        }
    }

    fn acquire(&self) {
        loop {
            let wait = {
                let mut state = self.state.lock().unwrap();
                let (ref mut tokens, ref mut updated) = *state;
                let now = Instant::now();
                *tokens = (*tokens + now.duration_since(*updated).as_secs_f64() * self.rate)
                    .min(self.capacity);
                *updated = now;
                if *tokens >= 1.0 {
                    *tokens -= 1.0;
                    return;
                }
                (1.0 - *tokens) / self.rate
            };
            std::thread::sleep(Duration::from_secs_f64(wait));
        }
    }
}

/// How a fetch treats the cache. `max_age: None` means any cached copy is
/// acceptable — correct for immutable archive documents. Endpoints that
/// change (submissions, company facts, the current day's index) should pass
/// an explicit `max_age`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FetchPolicy {
    pub max_age: Option<Duration>,
    pub force_refresh: bool,
    pub allow_404: bool,
}

impl FetchPolicy {
    pub fn max_age(age: Duration) -> Self {
        Self {
            max_age: Some(age),
            ..Self::default()
        }
    }
}

#[derive(Debug)]
pub struct Response {
    pub url: String,
    pub status: u16,
    pub body: Vec<u8>,
    pub fetched_at: DateTime<Utc>,
    pub from_cache: bool,
}

impl Response {
    pub fn json<T: DeserializeOwned>(&self) -> Result<T> {
        serde_json::from_slice(&self.body).map_err(|source| Error::Json {
            url: self.url.clone(),
            source,
        })
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Serialize, Deserialize)]
struct CacheMeta {
    url: String,
    status: u16,
    fetched_at: DateTime<Utc>,
}

pub struct EdgarClient {
    config: Config,
    limiter: RateLimiter,
    agent: ureq::Agent,
}

impl EdgarClient {
    pub fn new(config: Config) -> Result<Self> {
        config.ensure_dirs()?;
        let agent = ureq::AgentBuilder::new()
            .timeout(REQUEST_TIMEOUT)
            .user_agent(&config.user_agent)
            .build();
        Ok(Self {
            config,
            limiter: RateLimiter::new(SEC_RATE_LIMIT_PER_SEC),
            agent,
        })
    }

    // -- cache layout -------------------------------------------------------

    fn cache_paths(&self, url: &str) -> (PathBuf, PathBuf) {
        let digest = sha1_smol::Sha1::from(url.as_bytes()).digest().to_string();
        let shard = self.config.cache_dir.join(&digest[..2]);
        (
            shard.join(format!("{digest}.body.gz")),
            shard.join(format!("{digest}.meta.json")),
        )
    }

    fn read_cache(&self, url: &str, max_age: Option<Duration>) -> Option<Response> {
        let (body_path, meta_path) = self.cache_paths(url);
        let meta_text = std::fs::read_to_string(&meta_path).ok()?;
        let meta: CacheMeta = serde_json::from_str(&meta_text).ok()?;
        if let Some(max_age) = max_age {
            let age = Utc::now().signed_duration_since(meta.fetched_at);
            if age.to_std().map(|a| a > max_age).unwrap_or(true) {
                return None;
            }
        }
        let compressed = std::fs::read(&body_path).ok()?;
        let mut body = Vec::new();
        GzDecoder::new(compressed.as_slice())
            .read_to_end(&mut body)
            .ok()?;
        Some(Response {
            url: url.to_string(),
            status: meta.status,
            body,
            fetched_at: meta.fetched_at,
            from_cache: true,
        })
    }

    fn write_cache(
        &self,
        url: &str,
        status: u16,
        body: &[u8],
        fetched_at: DateTime<Utc>,
    ) -> Result<()> {
        use std::io::Write;
        let (body_path, meta_path) = self.cache_paths(url);
        let io_err = |path: &PathBuf| {
            let path = path.clone();
            move |source| Error::Io { path, source }
        };
        let shard = body_path.parent().expect("cache path has parent");
        std::fs::create_dir_all(shard).map_err(io_err(&body_path.clone()))?;

        // Write-then-rename so a crash mid-write cannot leave a truncated
        // body behind.
        let tmp = body_path.with_extension("tmp");
        let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body).map_err(io_err(&tmp))?;
        let compressed = encoder.finish().map_err(io_err(&tmp))?;
        std::fs::write(&tmp, compressed).map_err(io_err(&tmp))?;
        std::fs::rename(&tmp, &body_path).map_err(io_err(&body_path))?;

        let meta = CacheMeta {
            url: url.to_string(),
            status,
            fetched_at,
        };
        std::fs::write(
            &meta_path,
            serde_json::to_string_pretty(&meta).expect("meta serializes"),
        )
        .map_err(io_err(&meta_path))?;
        Ok(())
    }

    // -- fetching -----------------------------------------------------------

    /// Fetch `url`, preferring cache. Returns `Ok(None)` for a 404 when
    /// `allow_404` is set; not every company has every endpoint, and that is
    /// ordinary rather than exceptional.
    pub fn get(&self, url: &str, policy: FetchPolicy) -> Result<Option<Response>> {
        if !policy.force_refresh {
            if let Some(cached) = self.read_cache(url, policy.max_age) {
                if cached.status == 404 && policy.allow_404 {
                    return Ok(None);
                }
                if cached.status < 400 {
                    return Ok(Some(cached));
                }
            }
        }

        for attempt in 1..=MAX_ATTEMPTS {
            self.limiter.acquire();
            let fetched_at = Utc::now();
            let result = self.agent.get(url).call();
            let (status, body) = match result {
                Ok(resp) => {
                    let status = resp.status();
                    let mut body = Vec::new();
                    resp.into_reader()
                        .read_to_end(&mut body)
                        .map_err(|source| Error::Io {
                            path: PathBuf::from(url),
                            source,
                        })?;
                    (status, body)
                }
                Err(ureq::Error::Status(status, _)) => (status, Vec::new()),
                Err(transport) => {
                    // Connection-level failure: back off and retry.
                    eprintln!("attempt {attempt}/{MAX_ATTEMPTS} for {url} failed: {transport}");
                    if attempt == MAX_ATTEMPTS {
                        return Err(Error::Http {
                            url: url.to_string(),
                            source: Box::new(transport),
                        });
                    }
                    backoff(attempt);
                    continue;
                }
            };

            if status == 404 {
                self.write_cache(url, 404, &[], fetched_at)?;
                return if policy.allow_404 {
                    Ok(None)
                } else {
                    Err(Error::NotFound(url.to_string()))
                };
            }

            if RETRY_STATUSES.contains(&status) {
                eprintln!(
                    "status {status} for {url}; backing off (attempt {attempt}/{MAX_ATTEMPTS})"
                );
                backoff(attempt);
                continue;
            }

            if status >= 400 {
                return Err(Error::Status {
                    url: url.to_string(),
                    status,
                });
            }

            self.write_cache(url, status, &body, fetched_at)?;
            return Ok(Some(Response {
                url: url.to_string(),
                status,
                body,
                fetched_at,
                from_cache: false,
            }));
        }

        Err(Error::Exhausted {
            url: url.to_string(),
            attempts: MAX_ATTEMPTS,
        })
    }

    pub fn get_json<T: DeserializeOwned>(
        &self,
        url: &str,
        policy: FetchPolicy,
    ) -> Result<Option<T>> {
        match self.get(url, policy)? {
            None => Ok(None),
            Some(resp) => resp.json().map(Some),
        }
    }
}

fn backoff(attempt: u32) {
    let secs = 2u64.pow(attempt).min(30);
    std::thread::sleep(Duration::from_secs(secs));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_client() -> (EdgarClient, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            user_agent: "test test@example.com".into(),
            data_dir: dir.path().to_path_buf(),
            cache_dir: dir.path().join("cache"),
        };
        (EdgarClient::new(config).unwrap(), dir)
    }

    #[test]
    fn cache_round_trip_preserves_body_and_meta() {
        let (client, _dir) = test_client();
        let url = "https://example.com/x.json";
        let fetched_at = Utc::now();
        client
            .write_cache(url, 200, b"{\"ok\":true}", fetched_at)
            .unwrap();

        let cached = client.read_cache(url, None).expect("cache hit");
        assert_eq!(cached.status, 200);
        assert_eq!(cached.body, b"{\"ok\":true}");
        assert!(cached.from_cache);
    }

    #[test]
    fn stale_cache_misses_under_max_age() {
        let (client, _dir) = test_client();
        let url = "https://example.com/stale.json";
        let old = Utc::now() - chrono::Duration::hours(2);
        client.write_cache(url, 200, b"old", old).unwrap();

        assert!(client
            .read_cache(url, Some(Duration::from_secs(3600)))
            .is_none());
        assert!(client.read_cache(url, None).is_some());
    }

    #[test]
    fn cached_404_respects_allow_404() {
        let (client, _dir) = test_client();
        let url = "https://example.com/missing.json";
        client.write_cache(url, 404, b"", Utc::now()).unwrap();

        let policy = FetchPolicy {
            allow_404: true,
            ..FetchPolicy::default()
        };
        assert!(client.get(url, policy).unwrap().is_none());
    }

    #[test]
    fn rate_limiter_spaces_requests() {
        let limiter = RateLimiter::new(200.0);
        // Drain the burst capacity, then the next acquire must wait ~1/rate.
        for _ in 0..200 {
            limiter.acquire();
        }
        let start = Instant::now();
        limiter.acquire();
        assert!(start.elapsed() >= Duration::from_millis(3));
    }
}

use crate::error::Error;
use core::time::Duration;
use serde::Serialize;
use std::sync::OnceLock;
use std::thread::sleep;
use tracing::debug;
use ureq::Error::StatusCode;

static HTTP_CLIENT: OnceLock<ureq::Agent> = OnceLock::new();
static INGEST_URL: OnceLock<Option<String>> = OnceLock::new();
static INGEST_TOKEN: OnceLock<Option<String>> = OnceLock::new();

pub(crate) fn ingest_url() -> Result<&'static str, Error> {
    INGEST_URL
        .get_or_init(|| {
            std::env::var("SINGULARITY_INGEST_URL")
                .ok()
                .and_then(|value| nonempty_url(&value))
        })
        .as_deref()
        .ok_or(Error::IngestUrlNotConfigured)
}

fn nonempty_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn ingest_token() -> Option<&'static str> {
    INGEST_TOKEN
        .get_or_init(|| std::env::var("SINGULARITY_INGEST_TOKEN").ok())
        .as_deref()
}

/// Where match start and end reports go: `SINGULARITY_LIVE_URL`, or the
/// `/ingest/live` path next to a Singularity `.../ingest/salts` URL.
pub(crate) fn live_url() -> Option<String> {
    if let Ok(url) = std::env::var("SINGULARITY_LIVE_URL")
        && !url.trim().is_empty()
    {
        return Some(url);
    }
    live_url_for(&std::env::var("SINGULARITY_INGEST_URL").ok()?)
}

fn live_url_for(ingest_url: &str) -> Option<String> {
    let base = ingest_url
        .trim_end_matches('/')
        .strip_suffix("/ingest/salts")?;
    Some(format!("{base}/ingest/live"))
}

/// POSTs one JSON body with the collector token.
pub(crate) fn post_json<T: Serialize>(url: &str, body: &T) -> Result<(), String> {
    let agent = HTTP_CLIENT.get_or_init(ureq::Agent::new_with_defaults);
    let request = agent.post(url);
    let request = match ingest_token() {
        Some(token) => request.header("Authorization", format!("Bearer {token}")),
        None => request,
    };
    match request.send_json(body) {
        Ok(response) if response.status().is_success() => Ok(()),
        Ok(response) => Err(format!("HTTP {}", response.status())),
        Err(error) => Err(error.to_string()),
    }
}

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct Salts {
    pub(super) match_id: u64,
    pub(super) cluster_id: Option<u32>,
    pub(super) metadata_salt: Option<u32>,
    pub(super) replay_salt: Option<u32>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_username"
    )]
    pub(super) username: Option<u32>,
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::ref_option)]
fn serialize_username<S: serde::Serializer>(
    value: &Option<u32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(id) => serializer.serialize_str(&format!("ingest-tool:{id}")),
        None => serializer.serialize_none(),
    }
}

impl Salts {
    pub(crate) fn from_url(url: &str) -> Option<Self> {
        // Expect URLs like: http://replay404.valve.net/1422450/37959196_937530290.meta.bz2 or http://replay183.valve.net/1422450/42476710_428480166.dem.bz2
        // Strip query parameters if present
        let base_url = url.split_once('?').map_or(url, |(path, _)| path);

        let (cluster_str, remaining) = base_url
            .strip_prefix("http://replay")?
            .split_once(".valve.net/")?;
        // remaining should be like "1422450/37959196_937530290.meta.bz2"
        let name = remaining.rsplit_once('/').map(|(_, name)| name)?;
        if name.ends_with(".meta.bz2") {
            let name = name.strip_suffix(".meta.bz2")?;
            let (match_str, salt_str) = name.split_once('_')?;

            Some(Self {
                cluster_id: Some(cluster_str.parse().ok()?),
                match_id: match_str.parse().ok()?,
                metadata_salt: salt_str.parse().ok(),
                replay_salt: None,
                username: crate::steam_user::current_steam_id3(),
            })
        } else if name.ends_with(".dem.bz2") {
            let name = name.strip_suffix(".dem.bz2")?;
            let (match_str, salt_str) = name.split_once('_')?;

            Some(Self {
                cluster_id: Some(cluster_str.parse().ok()?),
                match_id: match_str.parse().ok()?,
                replay_salt: salt_str.parse().ok(),
                metadata_salt: None,
                username: crate::steam_user::current_steam_id3(),
            })
        } else {
            None
        }
    }

    pub(crate) fn ingest(&self) -> Result<(), Error> {
        if self.match_id > 10000000000 {
            return Err(Error::MatchIdTooLarge);
        }
        let url = ingest_url()?;

        let max_retries = 10;
        let mut attempt = 0;

        loop {
            attempt += 1;
            debug!("Ingesting salts: {self:?} (retry {attempt}/{max_retries})");
            let agent = HTTP_CLIENT.get_or_init(ureq::Agent::new_with_defaults);
            let response = match ingest_token() {
                Some(token) => agent
                    .post(url)
                    .header("Authorization", format!("Bearer {token}"))
                    .send_json([self]),
                None => agent.post(url).send_json([self]),
            };
            match response {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(mut resp) if attempt == max_retries => {
                    let text = resp.body_mut().read_to_string().unwrap_or_default();
                    return Err(Error::FailedToIngest(text));
                }
                Err(e) if attempt == max_retries || matches!(e, StatusCode(s) if s == 400) => {
                    return Err(Error::Ureq(e));
                }
                _ => sleep(Duration::from_secs(3)), // Retry on error
            }
        }
    }

    pub(crate) fn ingest_many(salts: &[Salts]) -> Result<(), Error> {
        let url = ingest_url()?;
        let max_retries = 10;
        let num_salts = salts.len();
        let mut attempt = 0;
        loop {
            attempt += 1;
            if attempt > 1 {
                debug!("Ingesting {num_salts} salts (retry {attempt}/{max_retries})");
            } else {
                debug!("Ingesting {num_salts} salts");
            }

            let agent = HTTP_CLIENT.get_or_init(ureq::Agent::new_with_defaults);
            let response = match ingest_token() {
                Some(token) => agent
                    .post(url)
                    .header("Authorization", format!("Bearer {token}"))
                    .send_json(salts),
                None => agent.post(url).send_json(salts),
            };
            match response {
                Ok(r) if r.status().is_success() => return Ok(()),
                Ok(mut resp) if attempt == max_retries => {
                    let text = resp.body_mut().read_to_string().unwrap_or_default();
                    return Err(Error::FailedToIngest(text));
                }
                Err(e) if attempt == max_retries || matches!(e, StatusCode(s) if s == 400) => {
                    return Err(Error::Ureq(e));
                }
                _ => sleep(Duration::from_secs(3)), // Retry on error
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_the_live_url_from_a_singularity_ingest_url() {
        assert_eq!(
            live_url_for("http://192.168.1.110:8080/ingest/salts").as_deref(),
            Some("http://192.168.1.110:8080/ingest/live")
        );
        assert_eq!(live_url_for("https://example.com/other/path"), None);
    }

    #[test]
    fn rejects_missing_or_blank_ingest_urls() {
        assert_eq!(nonempty_url(""), None);
        assert_eq!(nonempty_url("  \n"), None);
        assert_eq!(
            nonempty_url(" https://singularity.example/ingest/salts ").as_deref(),
            Some("https://singularity.example/ingest/salts")
        );
    }

    #[test]
    fn test_extract_salts() {
        #[allow(clippy::type_complexity)]
        let cases: &[(&str, u32, u64, Option<u32>, Option<u32>)] = &[
            (
                "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2",
                404,
                37959196,
                Some(937530290),
                None,
            ),
            (
                "http://replay400.valve.net/1422450/38090632_88648761.meta.bz2",
                400,
                38090632,
                Some(88648761),
                None,
            ),
            (
                "http://replay183.valve.net/1422450/42476710_428480166.meta.bz2",
                183,
                42476710,
                Some(428480166),
                None,
            ),
            (
                "http://replay183.valve.net/1422450/42476710_428480166.dem.bz2",
                183,
                42476710,
                None,
                Some(428480166),
            ),
            (
                "http://replay404.valve.net/1422450/37959196_937530290.meta.bz2?v=2",
                404,
                37959196,
                Some(937530290),
                None,
            ),
            (
                "http://replay183.valve.net/1422450/42476710_428480166.dem.bz2?v=2",
                183,
                42476710,
                None,
                Some(428480166),
            ),
        ];

        for &(url, cluster_id, match_id, metadata_salt, replay_salt) in cases {
            let salts = Salts::from_url(url).unwrap();
            assert_eq!(salts.cluster_id, Some(cluster_id));
            assert_eq!(salts.match_id, match_id);
            assert_eq!(salts.metadata_salt, metadata_salt);
            assert_eq!(salts.replay_salt, replay_salt);
        }
    }
}

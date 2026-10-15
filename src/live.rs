//! Reports when a Deadlock match starts and ends.
//!
//! With `-condebug` the game writes its console to `console.log`. When a match
//! is found it logs `Lobby <lobby> for Match <match id> created`, and when the
//! player leaves the match `... destroyed`. The platform uses the start report
//! to post both teams in Discord and the end report to fetch the match.

use core::time::Duration;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::thread::sleep;

use serde::Serialize;
use tracing::{debug, info, warn};

use crate::{game, launch_options, steam_user, utils};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const DISCOVERY_INTERVAL: Duration = Duration::from_secs(300);
const MAX_RETRIES: u32 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LobbyEvent {
    Started,
    Ended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct LiveReport {
    pub(crate) match_id: u64,
    pub(crate) event: LobbyEvent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) account_id: Option<u32>,
}

/// Reads one console line such as
/// `10/15 18:02:11 Lobby 29173 for Match 117420947 created`.
pub(crate) fn parse_line(line: &str) -> Option<(u64, LobbyEvent)> {
    let start = line.find("Lobby ")?;
    let mut words = line[start..].split_whitespace();
    words.next()?; // "Lobby"
    words.next()?; // lobby ID
    if !words.next()?.eq_ignore_ascii_case("for") || !words.next()?.eq_ignore_ascii_case("match") {
        return None;
    }
    let match_id = words.next()?.parse::<u64>().ok().filter(|id| *id > 0)?;
    let event = match words.next()?.to_ascii_lowercase().as_str() {
        "created" => LobbyEvent::Started,
        "destroyed" => LobbyEvent::Ended,
        _ => return None,
    };
    Some((match_id, event))
}

/// Follows a log file that the game recreates on every launch.
struct Tail {
    path: PathBuf,
    offset: u64,
    /// The first bytes of the file, to notice when the game starts a new log.
    head: Vec<u8>,
    partial: String,
}

const HEAD_BYTES: usize = 256;

fn read_head(file: &mut File) -> Vec<u8> {
    let mut head = Vec::with_capacity(HEAD_BYTES);
    let _ = file.seek(SeekFrom::Start(0));
    let _ = file.by_ref().take(HEAD_BYTES as u64).read_to_end(&mut head);
    head
}

impl Tail {
    /// Starts at the end of the file so old matches are not reported again.
    fn new(path: PathBuf) -> Self {
        let (offset, head) = File::open(&path).map_or((0, Vec::new()), |mut file| {
            let len = file.metadata().map_or(0, |meta| meta.len());
            (len, read_head(&mut file))
        });
        Self {
            path,
            offset,
            head,
            partial: String::new(),
        }
    }

    fn read_lines(&mut self) -> Vec<String> {
        let Ok(mut file) = File::open(&self.path) else {
            return Vec::new();
        };
        let len = file.metadata().map_or(0, |meta| meta.len());
        let head = read_head(&mut file);
        let same_log = head.len() >= self.head.len() && head[..self.head.len()] == self.head[..];
        if len < self.offset || !same_log {
            // The game started again and replaced the log.
            self.offset = 0;
            self.partial.clear();
        }
        if head.len() > self.head.len() || !same_log {
            self.head = head;
        }
        if len == self.offset || file.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut bytes = Vec::new();
        let Ok(read) = file.read_to_end(&mut bytes) else {
            return Vec::new();
        };
        self.offset += read as u64;
        self.partial.push_str(&String::from_utf8_lossy(&bytes));
        let Some(last_newline) = self.partial.rfind('\n') else {
            return Vec::new();
        };
        let rest = self.partial.split_off(last_newline + 1);
        let complete = core::mem::replace(&mut self.partial, rest);
        complete.lines().map(str::to_owned).collect()
    }
}

/// Watches the console log forever. Returns straight away when reporting is off.
pub(crate) fn watch() {
    let Some(url) = utils::live_url() else {
        info!("Live match reports are off: SINGULARITY_INGEST_URL is not a Singularity URL");
        return;
    };
    let path = loop {
        if let Some(path) = game::console_log_path() {
            break path;
        }
        debug!("Deadlock install not found yet; checking again in 5 minutes");
        sleep(DISCOVERY_INTERVAL);
    };
    info!("Watching Deadlock console log: {}", path.display());
    warn_without_console_log_flag();

    let mut tail = Tail::new(path);
    let mut last_sent: Option<(u64, LobbyEvent)> = None;
    loop {
        for line in tail.read_lines() {
            let Some((match_id, event)) = parse_line(&line) else {
                continue;
            };
            if last_sent == Some((match_id, event)) {
                continue;
            }
            last_sent = Some((match_id, event));
            let report = LiveReport {
                match_id,
                event,
                account_id: steam_user::current_steam_id3(),
            };
            info!("Match {match_id} {event:?}");
            if let Err(error) = send(&url, &report) {
                warn!("Failed to report match {match_id}: {error}");
            }
        }
        sleep(POLL_INTERVAL);
    }
}

fn warn_without_console_log_flag() {
    let (Ok(steam_dir), Some(account_id)) = (
        steamlocate::SteamDir::locate(),
        steam_user::current_steam_id3(),
    ) else {
        return;
    };
    let config = launch_options::local_config_path(steam_dir.path(), account_id);
    let has_flag = std::fs::read_to_string(&config)
        .ok()
        .and_then(|contents| launch_options::has_console_log_flag(&contents));
    if has_flag == Some(false) {
        warn!(
            "Deadlock's launch options do not include {}; match starts will not be seen. \
             Close Steam and run this program with --enable-console-log.",
            launch_options::CONSOLE_LOG_FLAG
        );
    }
}

fn send(url: &str, report: &LiveReport) -> Result<(), String> {
    let mut attempt = 0;
    loop {
        attempt += 1;
        match utils::post_json(url, report) {
            Ok(()) => return Ok(()),
            Err(error) if attempt >= MAX_RETRIES => return Err(error),
            Err(error) => {
                debug!("Live report failed (attempt {attempt}/{MAX_RETRIES}): {error}");
                sleep(Duration::from_secs(3));
            }
        }
    }
}

/// `--enable-console-log`: add `-condebug` for the Steam user who logged in last.
pub(crate) fn enable_console_log() -> Result<String, String> {
    let steam_dir =
        steamlocate::SteamDir::locate().map_err(|error| format!("Steam not found: {error}"))?;
    let account_id =
        steam_user::current_steam_id3().ok_or("no Steam user has logged in on this PC")?;
    let config = launch_options::local_config_path(steam_dir.path(), account_id);
    let contents = std::fs::read_to_string(&config)
        .map_err(|error| format!("cannot read {}: {error}", config.display()))?;
    match launch_options::add_console_log_flag(&contents)? {
        launch_options::Change::AlreadySet => Ok(format!(
            "Deadlock already launches with {}",
            launch_options::CONSOLE_LOG_FLAG
        )),
        launch_options::Change::Updated(updated) => {
            write_with_backup(&config, &contents, &updated)?;
            Ok(format!(
                "Added {} to Deadlock's launch options in {}",
                launch_options::CONSOLE_LOG_FLAG,
                config.display()
            ))
        }
    }
}

fn write_with_backup(path: &Path, original: &str, updated: &str) -> Result<(), String> {
    let backup = path.with_extension("vdf.singularity-backup");
    std::fs::write(&backup, original)
        .map_err(|error| format!("cannot write {}: {error}", backup.display()))?;
    std::fs::write(path, updated)
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lobby_lines() {
        assert_eq!(
            parse_line("10/15 18:02:11 Lobby 29173 for Match 117420947 created"),
            Some((117420947, LobbyEvent::Started))
        );
        assert_eq!(
            parse_line("[Client] Lobby 29173 for Match 117420947 destroyed"),
            Some((117420947, LobbyEvent::Ended))
        );
        assert_eq!(parse_line("Lobby 1 for Match 0 created"), None);
        assert_eq!(
            parse_line("[Hideout] Hideout Lobby Connection State: x (3)"),
            None
        );
        assert_eq!(parse_line("Lobby 1 for Match 42 updated"), None);
    }

    #[test]
    fn report_serializes_for_the_platform() {
        let report = LiveReport {
            match_id: 5,
            event: LobbyEvent::Started,
            account_id: Some(7),
        };
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"match_id":5,"event":"started","account_id":7}"#
        );
    }

    #[test]
    fn tail_reads_new_lines_and_handles_truncation() {
        let path = std::env::temp_dir().join(format!("singularity-tail-{}", std::process::id()));
        std::fs::write(&path, "old line\n").unwrap();
        let mut tail = Tail::new(path.clone());
        assert!(tail.read_lines().is_empty());

        let append = |text: &str| {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            file.write_all(text.as_bytes()).unwrap();
        };
        append("first\nsec");
        assert_eq!(tail.read_lines(), vec!["first"]);
        append("ond\n");
        assert_eq!(tail.read_lines(), vec!["second"]);

        std::fs::write(&path, "new\n").unwrap();
        assert_eq!(tail.read_lines(), vec!["new"]);

        // A new log that is already longer than the old one is still read from the start.
        std::fs::write(
            &path,
            "a much longer replacement log line\nwith two lines\n",
        )
        .unwrap();
        assert_eq!(
            tail.read_lines(),
            vec!["a much longer replacement log line", "with two lines"]
        );
        let _ = std::fs::remove_file(&path);
    }
}

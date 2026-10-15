use crate::ingestion_cache;
use crate::utils::Salts;
use core::time::Duration;
use memchr::{memchr, memmem};
use notify::event::{CreateKind, ModifyKind};
use notify::{EventKind, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Instant;
use tracing::{debug, info, warn};

const DEADLOCK_APP_ID: &str = "1422450";
const MAX_BYTES_TO_READ: usize = 200;
const SEARCH_SEQUENCE: &[u8; 10] = b".valve.net";
const PATH_END_MARKERS: [u8; 6] = *b" '\0\n\r\"";
const CACHE_RESCAN_INTERVAL: Duration = Duration::from_mins(5);

pub(super) fn scan_directory(dir: &Path, results: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                scan_directory(&path, results);
            } else if path.is_file()
                && let Some(url) = extract_replay_url(&path)
            {
                results.push(url);
            }
        }
    }
}

fn extract_replay_url(path: &Path) -> Option<String> {
    let Ok(mut file) = fs::File::open(path) else {
        return None;
    };
    let mut data = vec![0u8; MAX_BYTES_TO_READ];
    let bytes_read = file.read(&mut data).ok()?;
    data.truncate(bytes_read);

    let finder = memmem::Finder::new(SEARCH_SEQUENCE);

    // Find all occurrences of .valve.net
    for i in finder.find_iter(&data) {
        // Extract Host
        let host_start = (0..i)
            .rev()
            .find(|&pos| !data[pos].is_ascii_alphanumeric() && data[pos] != b'.')
            .map_or(0, |pos| pos + 1);
        let host_end = i + SEARCH_SEQUENCE.len();
        let host_slice = &data[host_start..host_end];

        let Ok(host) = core::str::from_utf8(host_slice) else {
            continue;
        };
        if !host.starts_with("replay") || !host.contains(".valve.net") {
            continue;
        }

        // Extract Path
        let path_start = match memchr(b'/', &data[host_end..]) {
            Some(slash_pos) => host_end + slash_pos,
            None => continue,
        };
        let path_slice = &data[path_start..];
        let path_end = PATH_END_MARKERS
            .into_iter()
            .filter_map(|marker| memchr(marker, path_slice))
            .min()?;

        let Ok(path) = core::str::from_utf8(&path_slice[..path_end]) else {
            continue;
        };
        if !path.contains(DEADLOCK_APP_ID) {
            continue;
        }

        // Construct full URL
        return Some(format!("http://{host}{path}"));
    }

    None
}

pub(super) fn initial_cache_dir_ingest(cache_dir: &Path) {
    debug!("Scanning cache directory: {}", cache_dir.display());
    let mut results = Vec::new();
    scan_directory(cache_dir, &mut results);
    let mut seen = HashSet::new();
    let salts = results
        .into_iter()
        .filter_map(|url| Salts::from_url(&url))
        .filter(|salt| is_new_salt(salt) && seen.insert(*salt))
        .collect::<Vec<_>>();

    if salts.is_empty() {
        return;
    }
    info!("Found {} new salts in Steam's HTTP cache", salts.len());

    match Salts::ingest_many(&salts) {
        Ok(..) => {
            // Mark all salts as successfully ingested in the shared cache
            for salt in &salts {
                ingestion_cache::mark_ingested(salt);
            }
        }
        Err(e) => warn!("Failed to ingest salts: {e:?}"),
    }
}

pub(super) fn watch_cache_dir(cache_dir: &Path) -> notify::Result<()> {
    debug!("Watching cache directory: {}", cache_dir.display());
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(cache_dir, RecursiveMode::Recursive)?;
    let mut next_rescan = Instant::now() + CACHE_RESCAN_INTERVAL;

    loop {
        let wait = next_rescan.saturating_duration_since(Instant::now());
        let event = match rx.recv_timeout(wait) {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => {
                warn!("Cache watcher error: {error:?}");
                continue;
            }
            Err(RecvTimeoutError::Timeout) => {
                debug!("Periodic Steam cache rescan");
                initial_cache_dir_ingest(cache_dir);
                next_rescan = Instant::now() + CACHE_RESCAN_INTERVAL;
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let is_data_modify = matches!(event.kind, EventKind::Modify(ModifyKind::Data(_)));
        let is_file_create = matches!(
            event.kind,
            EventKind::Create(CreateKind::Any | CreateKind::File)
        );
        if !is_data_modify && !is_file_create {
            continue;
        }
        for path in event.paths {
            if path.is_file()
                && let Some(url) = extract_replay_url(&path)
                && let Some(salts) = Salts::from_url(&url)
            {
                // Check if we've already ingested this salt using the shared cache
                if !is_new_salt(&salts) {
                    continue;
                }

                match salts.ingest() {
                    Ok(..) => {
                        info!("Ingested salts: {salts:?}");
                        ingestion_cache::mark_ingested(&salts);
                    }
                    Err(e) => warn!("Failed to ingest salts: {e:?}"),
                }
            }
        }
    }
    Ok(())
}

fn is_new_salt(salts: &Salts) -> bool {
    (salts.metadata_salt.is_some() && !ingestion_cache::is_ingested(salts.match_id, true))
        || (salts.replay_salt.is_some() && !ingestion_cache::is_ingested(salts.match_id, false))
}

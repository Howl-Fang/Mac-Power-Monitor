//! Always-on rolling history in /tmp, retained for 48 hours.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::history::raw;
use crate::model::{Sample, Snapshot, SystemInfo};

pub const RETENTION_SECS: u64 = 48 * 60 * 60;

pub fn default_base_dir() -> PathBuf {
    PathBuf::from("/tmp/macpower")
}

pub struct HistoryStore {
    history_dir: PathBuf,
    recordings_dir: PathBuf,
    hour: Option<i64>,
    writer: Option<BufWriter<File>>,
}

impl HistoryStore {
    pub fn open(root: PathBuf) -> std::io::Result<Self> {
        let history_dir = root.join("history");
        let recordings_dir = root.join("recordings");
        fs::create_dir_all(&history_dir)?;
        fs::create_dir_all(&recordings_dir)?;
        secure_dir(&root);
        secure_dir(&history_dir);
        secure_dir(&recordings_dir);
        let mut store = Self {
            history_dir,
            recordings_dir,
            hour: None,
            writer: None,
        };
        store.prune(epoch_hour(SystemTime::now()));
        Ok(store)
    }

    pub fn recordings_dir(&self) -> &Path {
        &self.recordings_dir
    }

    pub fn record(&mut self, snapshot: &Snapshot, system: &SystemInfo) -> std::io::Result<()> {
        let hour = epoch_hour(snapshot.sample.ts);
        if self.hour != Some(hour) {
            self.close_writer();
            let path = self.history_dir.join(format!("history-{hour}.tsv"));
            let is_new = fs::metadata(&path)
                .map(|metadata| metadata.len() == 0)
                .unwrap_or(true);
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            let mut writer = BufWriter::new(file);
            if is_new {
                raw::write_history_header(&mut writer, system)?;
            }
            self.writer = Some(writer);
            self.hour = Some(hour);
            self.prune(hour);
        }
        let writer = self
            .writer
            .as_mut()
            .expect("history writer is opened with the current hour");
        writeln!(writer, "{}", raw::encode_compact_row(snapshot))?;
        writer.flush()
    }

    pub fn flush(&mut self) {
        if let Some(writer) = self.writer.as_mut() {
            let _ = writer.flush();
        }
    }

    pub fn latest_timestamp(&self) -> Option<SystemTime> {
        self.files()
            .into_iter()
            .rev()
            .find_map(|path| raw::last_timestamp(&path))
    }

    pub fn oldest_timestamp(&self) -> Option<SystemTime> {
        self.files()
            .into_iter()
            .find_map(|path| raw::first_timestamp(&path))
    }

    pub fn samples_since(&mut self, secs: Option<u64>) -> Result<Vec<Sample>, String> {
        self.flush();
        let latest = self
            .latest_timestamp()
            .ok_or_else(|| "no rolling history samples yet".to_string())?;
        let cutoff = cutoff_from(latest, secs);
        let mut out = Vec::new();
        let mut last_error = None;
        for path in self.files_since(cutoff) {
            match raw::parse_file(&path) {
                Ok(samples) => out.extend(
                    samples
                        .into_iter()
                        .filter(|sample| cutoff.is_none_or(|cutoff| sample.ts >= cutoff)),
                ),
                Err(err) => last_error = Some(err),
            }
        }
        out.sort_by_key(|sample| sample.ts);
        if out.is_empty() {
            return Err(last_error.unwrap_or_else(|| "no samples in selected range".to_string()));
        }
        Ok(out)
    }

    pub fn export_tsv(
        &mut self,
        output: &Path,
        secs: Option<u64>,
        source_label: &str,
    ) -> Result<usize, String> {
        self.flush();
        let latest = self
            .latest_timestamp()
            .ok_or_else(|| "no rolling history samples yet".to_string())?;
        let oldest = self.oldest_timestamp();
        let cutoff = match (cutoff_from(latest, secs), oldest) {
            (Some(requested), Some(oldest)) => Some(requested.max(oldest)),
            (Some(requested), None) => Some(requested),
            (None, Some(oldest)) => Some(oldest),
            (None, None) => None,
        };
        let files = self.files_since(cutoff);
        raw::export_tsv_files(&files, output, cutoff, Some(latest), source_label)
    }

    pub fn list_recordings(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&self.recordings_dir) else {
            return Vec::new();
        };
        let mut recordings: Vec<(SystemTime, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let name = path.file_name()?.to_str()?;
                if !name.starts_with("recording-")
                    || !(name.ends_with(".tsv") || name.ends_with(".tsv.part"))
                {
                    return None;
                }
                let modified = entry
                    .metadata()
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(UNIX_EPOCH);
                Some((modified, path))
            })
            .collect();
        recordings.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
        recordings.into_iter().map(|(_, path)| path).collect()
    }

    fn close_writer(&mut self) {
        if let Some(mut writer) = self.writer.take() {
            let _ = writer.flush();
        }
    }

    fn files(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&self.history_dir) else {
            return Vec::new();
        };
        let mut files: Vec<(i64, PathBuf)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                let hour = hour_of_path(&path)?;
                Some((hour, path))
            })
            .collect();
        files.sort_by_key(|(hour, _)| *hour);
        files.into_iter().map(|(_, path)| path).collect()
    }

    fn files_since(&self, cutoff: Option<SystemTime>) -> Vec<PathBuf> {
        let Some(cutoff) = cutoff else {
            return self.files();
        };
        let cutoff_hour = epoch_hour(cutoff);
        self.files()
            .into_iter()
            .filter(|path| hour_of_path(path).is_some_and(|hour| hour >= cutoff_hour))
            .collect()
    }

    fn prune(&mut self, current_hour: i64) {
        let Ok(entries) = fs::read_dir(&self.history_dir) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let Some(hour) = hour_of_path(&path) else {
                continue;
            };
            // Keep the current partial hour plus the previous 48 hours;
            // exports trim the oldest bucket precisely to the requested range.
            if hour < current_hour - RETENTION_SECS as i64 / 3600 {
                let _ = fs::remove_file(path);
            }
        }
    }
}

impl Drop for HistoryStore {
    fn drop(&mut self) {
        self.close_writer();
    }
}

fn secure_dir(path: &Path) {
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

fn epoch_hour(time: SystemTime) -> i64 {
    (time
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
        / 3600) as i64
}

fn cutoff_from(latest: SystemTime, secs: Option<u64>) -> Option<SystemTime> {
    secs.map(|secs| {
        latest
            .checked_sub(Duration::from_secs(secs))
            .unwrap_or(UNIX_EPOCH)
    })
}

fn hour_of_path(path: &Path) -> Option<i64> {
    let name = path.file_name()?.to_str()?;
    let hour = name.strip_prefix("history-")?.strip_suffix(".tsv")?;
    hour.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_root(tag: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        std::env::temp_dir().join(format!("macpower-store-{tag}-{nonce}"))
    }

    fn snapshot(ts: SystemTime, system_w: f32) -> Snapshot {
        Snapshot {
            sample: Sample {
                ts,
                system_w: Some(system_w),
                battery_pct: Some(50.0),
                ..Sample::default()
            },
            rails: Vec::new(),
            battery: crate::model::BatteryInfo::default(),
            smc: crate::model::SmcValues::default(),
            system: SystemInfo::default(),
        }
    }

    #[test]
    fn records_compact_history_and_filters_by_time() {
        let root = unique_root("filter");
        let mut store = HistoryStore::open(root.clone()).unwrap();
        let now = SystemTime::now();
        let system = SystemInfo::default();
        store
            .record(&snapshot(now - Duration::from_secs(120), 1.0), &system)
            .unwrap();
        store
            .record(&snapshot(now - Duration::from_secs(30), 2.0), &system)
            .unwrap();

        let samples = store.samples_since(Some(60)).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].system_w, Some(2.0));

        let latest = store
            .files()
            .into_iter()
            .find_map(|path| fs::read_to_string(path).ok())
            .unwrap();
        assert!(latest.contains("# macpower history v1"));
        assert!(latest.contains("timestamp_ms"));
        assert!(!latest.contains("rails"));

        let output = root.join("export.tsv");
        let rows = store.export_tsv(&output, Some(60), "test").unwrap();
        assert_eq!(rows, 1);
        let text = fs::read_to_string(&output).unwrap();
        assert!(text.contains("# source=test"));
        assert!(text.contains("# range="));
        assert!(!text.contains("# range=all available"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn open_prunes_files_older_than_retention() {
        let root = unique_root("prune");
        let history = root.join("history");
        fs::create_dir_all(&history).unwrap();
        let current = epoch_hour(SystemTime::now());
        let old = history.join(format!("history-{}.tsv", current - 60));
        let recent = history.join(format!("history-{}.tsv", current - 1));
        fs::write(&old, "old").unwrap();
        fs::write(&recent, "recent").unwrap();

        let _store = HistoryStore::open(root.clone()).unwrap();
        assert!(!old.exists());
        assert!(recent.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn lists_recording_segments_newest_first() {
        let root = unique_root("recordings");
        let store = HistoryStore::open(root.clone()).unwrap();
        let first = store.recordings_dir().join("recording-1.tsv");
        let second = store.recordings_dir().join("recording-2.tsv.part");
        fs::write(&first, "").unwrap();
        std::thread::sleep(Duration::from_millis(10));
        fs::write(&second, "").unwrap();
        let recordings = store.list_recordings();
        assert_eq!(recordings.len(), 2);
        assert_eq!(recordings[0], second);
        let _ = fs::remove_dir_all(root);
    }
}

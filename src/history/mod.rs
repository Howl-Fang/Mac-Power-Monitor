pub mod pdf;
pub mod raw;
pub mod store;

use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

use crate::model::Sample;

pub struct History {
    samples: VecDeque<Sample>,
    max_duration: Duration,
}

impl History {
    pub fn new(max_duration: Duration) -> Self {
        Self {
            samples: VecDeque::new(),
            max_duration,
        }
    }

    pub fn push(&mut self, sample: Sample) {
        let newest = sample.ts;
        self.samples.push_back(sample);
        let cutoff = newest
            .checked_sub(self.max_duration)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        while let Some(front) = self.samples.front() {
            if front.ts < cutoff {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn samples(&self) -> impl Iterator<Item = &Sample> {
        self.samples.iter()
    }

    pub fn all(&self) -> Vec<&Sample> {
        self.samples.iter().collect()
    }

    pub fn window(&self, duration: Duration) -> Vec<&Sample> {
        let Some(latest) = self.samples.back() else {
            return Vec::new();
        };
        let cutoff = latest
            .ts
            .checked_sub(duration)
            .unwrap_or(SystemTime::UNIX_EPOCH);
        self.samples.iter().filter(|s| s.ts >= cutoff).collect()
    }

    pub fn summary(&self, duration: Option<Duration>) -> Summary {
        let samples = match duration {
            Some(d) => self.window(d),
            None => self.all(),
        };
        Summary::from_samples(&samples)
    }
}

#[derive(Clone, Debug, Default)]
pub struct MetricStats {
    pub min: Option<f32>,
    pub avg: Option<f32>,
    pub max: Option<f32>,
    pub energy_wh: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub count: usize,
    pub start: Option<SystemTime>,
    pub end: Option<SystemTime>,
    pub system: MetricStats,
    pub battery: MetricStats,
    pub cpu: MetricStats,
    pub ecpu: MetricStats,
    pub pcpu: MetricStats,
    pub gpu: MetricStats,
    pub ane: MetricStats,
    pub dram: MetricStats,
    pub display: MetricStats,
    pub other: MetricStats,
}

impl Summary {
    pub fn from_samples(samples: &[&Sample]) -> Self {
        let mut out = Self {
            count: samples.len(),
            start: samples.first().map(|s| s.ts),
            end: samples.last().map(|s| s.ts),
            ..Default::default()
        };
        out.system = stats(samples, |s| s.system_w);
        out.battery = stats(samples, |s| s.battery_w);
        out.cpu = stats(samples, |s| s.cpu_w);
        out.ecpu = stats(samples, |s| s.ecpu_w);
        out.pcpu = stats(samples, |s| s.pcpu_w);
        out.gpu = stats(samples, |s| s.gpu_w);
        out.ane = stats(samples, |s| s.ane_w);
        out.dram = stats(samples, |s| s.dram_w);
        out.display = stats(samples, |s| s.display_w);
        out.other = stats(samples, |s| s.other_w);
        out
    }

    pub fn duration_secs(&self) -> Option<u64> {
        match (self.start, self.end) {
            (Some(a), Some(b)) => b.duration_since(a).ok().map(|d| d.as_secs()),
            _ => None,
        }
    }
}

fn stats(samples: &[&Sample], accessor: impl Fn(&Sample) -> Option<f32>) -> MetricStats {
    let mut min: Option<f32> = None;
    let mut max: Option<f32> = None;
    let mut sum = 0.0f64;
    let mut count = 0usize;
    let mut energy_wh = 0.0f64;
    let mut prev: Option<(SystemTime, f32)> = None;

    for sample in samples {
        let Some(value) = accessor(sample).filter(|v| v.is_finite()) else {
            prev = None;
            continue;
        };
        min = Some(min.map_or(value, |m| m.min(value)));
        max = Some(max.map_or(value, |m| m.max(value)));
        sum += value as f64;
        count += 1;

        if let Some((prev_ts, prev_value)) = prev {
            let dt_hours = sample
                .ts
                .duration_since(prev_ts)
                .unwrap_or(Duration::ZERO)
                .as_secs_f64()
                / 3600.0;
            energy_wh += (prev_value as f64 + value as f64) * 0.5 * dt_hours;
        }
        prev = Some((sample.ts, value));
    }

    MetricStats {
        min,
        avg: (count > 0).then_some((sum / count as f64) as f32),
        max,
        energy_wh,
    }
}

/// Bucket samples into `count` columns, returning (x, avg, min, max).
pub fn series_minmax(
    samples: &[&Sample],
    count: usize,
    accessor: impl Fn(&Sample) -> Option<f32>,
) -> Vec<(f64, f64, f64, f64)> {
    if samples.is_empty() || count == 0 {
        return Vec::new();
    }
    let count = count.min(samples.len());
    let mut out = Vec::with_capacity(count);
    for bucket in 0..count {
        let start = bucket * samples.len() / count;
        let end = ((bucket + 1) * samples.len() / count)
            .max(start + 1)
            .min(samples.len());
        let mut sum = 0.0f64;
        let mut n = 0usize;
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for sample in &samples[start..end] {
            if let Some(v) = accessor(sample).filter(|v| v.is_finite()) {
                let v = v as f64;
                sum += v;
                n += 1;
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        if n == 0 {
            continue;
        }
        let x = (start + end) as f64 * 0.5 / samples.len() as f64;
        out.push((x, sum / n as f64, lo, hi));
    }
    out
}

/// Bucket samples into `count` columns, returning (x, avg) for a single series.
pub fn series_avg(
    samples: &[&Sample],
    count: usize,
    accessor: impl Fn(&Sample) -> Option<f32>,
) -> Vec<(f64, f64)> {
    series_minmax(samples, count, accessor)
        .into_iter()
        .map(|(x, avg, _, _)| (x, avg))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ts_secs: u64, system_w: f32) -> Sample {
        Sample {
            ts: SystemTime::UNIX_EPOCH + Duration::from_secs(ts_secs),
            system_w: Some(system_w),
            ..Sample::default()
        }
    }

    #[test]
    fn trims_by_duration() {
        let mut history = History::new(Duration::from_secs(10));
        for i in 0..20 {
            history.push(sample(i, i as f32));
        }
        assert_eq!(history.len(), 11);
        assert!(
            history.samples().next().unwrap().ts >= SystemTime::UNIX_EPOCH + Duration::from_secs(9)
        );
    }

    #[test]
    fn computes_stats_and_energy() {
        let samples = [sample(0, 1.0), sample(3600, 3.0)];
        let refs: Vec<&Sample> = samples.iter().collect();
        let summary = Summary::from_samples(&refs);
        assert_eq!(summary.system.min, Some(1.0));
        assert_eq!(summary.system.max, Some(3.0));
        assert_eq!(summary.system.avg, Some(2.0));
        assert!((summary.system.energy_wh - 2.0).abs() < 1e-9);
    }

    #[test]
    fn downsampling_preserves_extremes() {
        let samples: Vec<Sample> = (0..100).map(|i| sample(i, (i % 10) as f32)).collect();
        let refs: Vec<&Sample> = samples.iter().collect();
        let series = series_minmax(&refs, 10, |s| s.system_w);
        assert_eq!(series.len(), 10);
        let min = series
            .iter()
            .map(|(_, _, lo, _)| *lo)
            .fold(f64::INFINITY, f64::min);
        let max = series
            .iter()
            .map(|(_, _, _, hi)| *hi)
            .fold(f64::NEG_INFINITY, f64::max);
        assert_eq!(min, 0.0);
        assert_eq!(max, 9.0);
    }

    #[test]
    fn gaps_break_energy_integration() {
        let mut a = sample(0, 1.0);
        a.system_w = Some(1.0);
        let mut b = sample(1, 1.0);
        b.system_w = None;
        let mut c = sample(2, 3.0);
        c.system_w = Some(3.0);
        let samples = [a, b, c];
        let refs: Vec<&Sample> = samples.iter().collect();
        let summary = Summary::from_samples(&refs);
        assert_eq!(summary.system.energy_wh, 0.0);
        assert_eq!(summary.system.avg, Some(2.0));
    }
}

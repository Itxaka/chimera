//! Host-side per-VM metrics from /proc/<pid>. No guest agent, no ch API.

use serde::Serialize;
use std::time::Instant;

#[derive(Debug, Clone, Serialize)]
pub struct VmMetrics {
    pub cpu_pct: f32,
    pub rss_bytes: u64,
}

/// Sum of utime+stime (clock ticks) from a `/proc/<pid>/stat` line. The 2nd
/// field (comm) is wrapped in parens and may contain spaces/parens, so split
/// after the LAST ')': the remaining whitespace fields begin at `state` (the
/// 3rd overall field), making utime the 12th and stime the 13th of them.
pub fn parse_proc_stat_ticks(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    let rest = stat.get(close + 1..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // rest[0]=state(3) ... utime=overall 14 => rest index 11; stime=15 => 12.
    let utime: u64 = fields.get(11)?.parse().ok()?;
    let stime: u64 = fields.get(12)?.parse().ok()?;
    Some(utime + stime)
}

/// Resident set size in bytes from `/proc/<pid>/statm` (2nd field = resident
/// pages) × page_size.
pub fn parse_proc_statm_rss(statm: &str, page_size: u64) -> Option<u64> {
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * page_size)
}

fn clk_tck() -> u64 {
    nix::unistd::sysconf(nix::unistd::SysconfVar::CLK_TCK)
        .ok()
        .flatten()
        .map(|v| v as u64)
        .unwrap_or(100)
}

fn page_size() -> u64 {
    nix::unistd::sysconf(nix::unistd::SysconfVar::PAGE_SIZE)
        .ok()
        .flatten()
        .map(|v| v as u64)
        .unwrap_or(4096)
}

#[derive(Default)]
pub struct CpuSampler {
    last: Option<(u64, Instant)>, // (ticks, when)
}

impl CpuSampler {
    /// Read /proc/<pid>/{stat,statm}; CPU% from the delta vs the previous
    /// sample (0.0 on the first call). None if the process is gone/unreadable.
    pub fn sample(&mut self, pid: u32) -> Option<VmMetrics> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
        let ticks = parse_proc_stat_ticks(&stat)?;
        let rss_bytes = parse_proc_statm_rss(&statm, page_size())?;
        let now = Instant::now();
        let cpu_pct = match self.last {
            Some((prev_ticks, prev_when)) => {
                let elapsed = now.duration_since(prev_when).as_secs_f64();
                if elapsed > 0.0 {
                    let dticks = ticks.saturating_sub(prev_ticks) as f64;
                    ((dticks / clk_tck() as f64) / elapsed * 100.0) as f32
                } else {
                    0.0
                }
            }
            None => 0.0,
        };
        self.last = Some((ticks, now));
        Some(VmMetrics { cpu_pct, rss_bytes })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_ticks_handles_paren_comm() {
        // comm = "(cloud (hyp) visor)" with spaces+parens; utime=100 stime=50.
        // fields after last ')': state ppid pgrp sid tty tpgid flags minflt
        //   cminflt majflt cmajflt utime stime ...
        let stat = "4242 (cloud (hyp) visor) S 1 4242 4242 0 -1 0 0 0 0 0 100 50 0 0 20 0 1 0";
        assert_eq!(parse_proc_stat_ticks(stat), Some(150));
    }

    #[test]
    fn statm_rss_pages_times_pagesize() {
        // statm: size resident shared text lib data dt
        assert_eq!(
            parse_proc_statm_rss("12345 48 20 1 0 30 0", 4096),
            Some(48 * 4096)
        );
    }

    #[test]
    fn parsers_reject_garbage() {
        assert_eq!(parse_proc_stat_ticks("no parens here"), None);
        assert_eq!(parse_proc_statm_rss("", 4096), None);
    }

    #[test]
    fn first_sample_has_zero_cpu_then_reads_self() {
        // Sample our own pid twice; cpu_pct is finite and rss > 0.
        let mut s = CpuSampler::default();
        let pid = std::process::id();
        let m1 = s.sample(pid).expect("first sample");
        assert_eq!(m1.cpu_pct, 0.0);
        assert!(m1.rss_bytes > 0);
        let m2 = s.sample(pid).expect("second sample");
        assert!(m2.cpu_pct >= 0.0 && m2.cpu_pct.is_finite());
    }
}

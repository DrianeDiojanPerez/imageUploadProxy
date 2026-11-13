use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use jiff::tz::TimeZone;
use jiff::Timestamp;
use reqwest::header::HeaderMap;

use crate::repository::Usage;

/// What we know about how much of the api budget has gone.
///
/// GitHub sends the real numbers back on every response, so this mostly
/// remembers the last thing it told us. Counting locally would drift the
/// moment anything else used the same token.
#[derive(Default)]
pub struct UsageTracker {
    calls_since_boot: AtomicU64,
    calls_this_window: AtomicU64,
    last_call_at: AtomicU64,

    // What we believe has gone from the budget. GitHub answers reads and
    // writes from separate counters that each know only their own spending,
    // so this is kept in step with all of them rather than being any one of
    // their figures.
    window: Mutex<Option<Window>>,

    // Counted across every run rather than this one, which only means
    // anything when there is a file behind it to carry the number over.
    calls_total: AtomicU64,
    file: Option<PathBuf>,
    writing: Mutex<()>,
}

/// The hour we are in, and how much of the budget has gone in it.
///
/// What is left is worked out from these rather than taken off the wire,
/// since no single counter knows what the others have spent.
#[derive(Clone, Default, Deserialize, Serialize)]
struct Window {
    resets_at: u64,
    limit: u64,
    used: u64,
    resource: Option<String>,
}

/// What is worth surviving a restart.
///
/// GitHub's own figures are kept too, but only as the last thing it said
/// rather than as the truth. They are reported with the time they were
/// heard, and dropped outright once the window they belong to has passed.
///
/// Missing fields fall back to their default rather than throwing the file
/// out, so a file written by an older build still gives up its running total
/// instead of the count starting again from nothing.
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct Persisted {
    calls_total: u64,
    calls_this_window: u64,
    last_call_at: u64,
    window: Option<Window>,
}

impl UsageTracker {
    /// Build a tracker, picking up where a previous run left off.
    ///
    /// A missing or unreadable file is not worth refusing to start over,
    /// so it is reported and the count simply begins again.
    pub fn new(file: Option<PathBuf>) -> Self {
        let tracker = Self {
            file,
            ..Self::default()
        };

        let Some(path) = tracker.file.as_ref() else {
            return tracker;
        };

        match std::fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<Persisted>(&raw) {
                Ok(held) => {
                    tracker.calls_total.store(held.calls_total, Ordering::Relaxed);
                    tracker.last_call_at.store(held.last_call_at, Ordering::Relaxed);

                    // Only stand behind the saved figures while the window
                    // they came from is still running. Past that they are
                    // history, and the free look at GitHub replaces them.
                    let live = held.window.filter(|window| window.resets_at > now());

                    if live.is_some() {
                        tracker
                            .calls_this_window
                            .store(held.calls_this_window, Ordering::Relaxed);
                    }

                    tracing::info!(
                        path = %path.display(),
                        calls_total = held.calls_total,
                        window_still_live = live.is_some(),
                        "picked the usage count back up"
                    );

                    if let Ok(mut window) = tracker.window.lock() {
                        *window = live;
                    }
                }
                Err(error) => tracing::warn!(%error, path = %path.display(), "could not read the usage file, starting the count over"),
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(path = %path.display(), "no usage file yet, starting a fresh count");
            }
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "could not open the usage file, starting the count over")
            }
        }

        tracker
    }

    /// Write our own counting out, leaving the old file alone until the new
    /// one is whole, so a crash mid write cannot leave a half a file behind.
    fn save(&self) {
        let Some(path) = self.file.as_ref() else {
            return;
        };

        let held = Persisted {
            calls_total: self.calls_total.load(Ordering::Relaxed),
            calls_this_window: self.calls_this_window.load(Ordering::Relaxed),
            last_call_at: self.last_call_at.load(Ordering::Relaxed),
            window: self.window.lock().ok().and_then(|held| held.clone()),
        };

        let Ok(_writing) = self.writing.lock() else {
            return;
        };

        let temporary = path.with_extension("tmp");

        let written = serde_json::to_vec_pretty(&held)
            .map_err(std::io::Error::other)
            .and_then(|body| std::fs::write(&temporary, body))
            .and_then(|_| std::fs::rename(&temporary, path));

        if let Err(error) = written {
            tracing::warn!(%error, path = %path.display(), "could not write the usage file");
        }
    }

    /// Fold a response's rate limit headers into what we hold.
    ///
    /// Pass counted as false for calls that GitHub does not charge us for,
    /// so a look at the budget does not show up as spending it.
    pub fn record(&self, headers: &HeaderMap, counted: bool) {
        if counted {
            self.calls_since_boot.fetch_add(1, Ordering::Relaxed);
            self.calls_total.fetch_add(1, Ordering::Relaxed);
        }

        // GitHub keeps a separate budget, and a separate reset, for each
        // resource. Two calls landing in different ones is the usual reason
        // the reset looks like it moves about, so the raw headers are worth
        // a line when anyone goes looking.
        tracing::debug!(
            resource = text(headers, "x-ratelimit-resource").unwrap_or_else(|| "none".to_string()),
            limit = text(headers, "x-ratelimit-limit").unwrap_or_default(),
            used = text(headers, "x-ratelimit-used").unwrap_or_default(),
            remaining = text(headers, "x-ratelimit-remaining").unwrap_or_default(),
            reset = text(headers, "x-ratelimit-reset").unwrap_or_default(),
            counted,
            "github reported its rate limit"
        );

        self.last_call_at.store(now(), Ordering::Relaxed);

        // Without the reset there is no telling which hour the reply belongs
        // to, and everything below turns on that.
        let Some(resets_at) = number(headers, "x-ratelimit-reset") else {
            // A reply with no rate limit headers still cost us a call, so
            // the count is written out before dropping the rest.
            self.save();

            return;
        };

        let reported = number(headers, "x-ratelimit-used").unwrap_or_default();
        let limit = number(headers, "x-ratelimit-limit").unwrap_or_default();
        let resource = text(headers, "x-ratelimit-resource");
        let spent = u64::from(counted);

        // The lock is let go before saving, since save reads the same window
        // back and a second lock on the same thread would sit there forever.
        if let Ok(mut held) = self.window.lock() {
            let now = now();

            // Nothing held, or the hour we were in has been handed back, so
            // this reply is the start of a fresh one.
            let starting_over = held.as_ref().map_or(true, |window| window.resets_at <= now);

            if starting_over {
                *held = Some(Window {
                    resets_at,
                    limit,
                    used: reported.max(spent),
                    resource,
                });

                self.calls_this_window.store(spent, Ordering::Relaxed);
            } else if let Some(window) = held.as_mut() {
                // Our own call moves the count on by one whichever counter
                // answered it, and anything a counter reports beyond that is
                // spending we did not do. The larger of the two is the truth,
                // so a counter that happens to be behind the one we were
                // following never drags the count back, and never has to
                // catch up before it tells us anything.
                window.used = (window.used + spent).max(reported);

                // The soonest hand back is the one to quote, and taking the
                // earlier of the two keeps it from moving about as the
                // counters take turns answering.
                if resets_at > now {
                    window.resets_at = window.resets_at.min(resets_at);
                }

                if limit > 0 {
                    window.limit = limit;
                }

                if resource.is_some() {
                    window.resource = resource;
                }

                self.calls_this_window.fetch_add(spent, Ordering::Relaxed);
            }
        }

        self.save();
    }

    // Only worth reporting when it means something more than the count
    // since boot, which is to say when there is a file behind it.
    fn persisted_total(&self) -> Option<u64> {
        self.file.as_ref()?;

        Some(self.calls_total.load(Ordering::Relaxed))
    }

    /// Whether what we hold is worth reporting, or too old to mean anything.
    ///
    /// Nothing live means every budget we knew about has since been handed
    /// back, and the free look at GitHub is what fills them in again.
    pub fn is_stale(&self) -> bool {
        let Ok(held) = self.window.lock() else {
            return true;
        };

        held.as_ref().map_or(true, |window| window.resets_at <= now())
    }

    pub fn snapshot(&self) -> Usage {
        let calls_since_boot = self.calls_since_boot.load(Ordering::Relaxed);
        let calls_total = self.persisted_total();

        let nothing_known = Usage {
            calls_since_boot,
            calls_total,
            ..Usage::default()
        };

        let Ok(held) = self.window.lock() else {
            return nothing_known;
        };

        // An hour that has run out is one GitHub has already handed back, so
        // reporting it would mean claiming a shortage that no longer exists.
        let Some(window) = held.as_ref().filter(|window| window.resets_at > now()) else {
            return nothing_known;
        };

        let last_call_at = self.last_call_at.load(Ordering::Relaxed);
        let resets_in_seconds = window.resets_at - now();
        let percent_used = percentage(window.used, window.limit);

        Usage {
            limit: Some(window.limit),
            used: Some(window.used),
            remaining: Some(window.limit.saturating_sub(window.used)),
            percent_used,
            percent_remaining: percent_used.map(|percent| round(100.0 - percent)),
            resets_at: Some(window.resets_at),
            resets_at_utc: to_zone_string(window.resets_at, &TimeZone::UTC),
            resets_in_seconds: Some(resets_in_seconds),
            resets_in: Some(to_duration_string(resets_in_seconds)),
            resource: window.resource.clone(),
            calls_this_window: self.calls_this_window.load(Ordering::Relaxed),
            calls_since_boot,
            calls_total,
            last_call_at: Some(last_call_at),
            last_call_ago: Some(to_duration_string(now().saturating_sub(last_call_at))),
        }
    }
}

/// Spell an epoch second out as Wednesday, August 12th, 2026 3:32:01pm CST.
///
/// GitHub only ever sends the number, and a number is no use to anyone
/// reading the endpoint by eye. Whether that moment reads as CST or CDT is
/// the zone's business, which is the whole reason a real zone is carried
/// here rather than a plain offset from UTC.
pub fn to_zone_string(epoch: u64, zone: &TimeZone) -> Option<String> {
    let moment = Timestamp::from_second(epoch as i64)
        .ok()?
        .to_zoned(zone.clone());

    // strftime has no ordinal, so the day is spliced in between two halves
    // of the same format rather than the whole thing being built by hand.
    Some(format!(
        "{}{}, {}",
        moment.strftime("%A, %B %-d"),
        ordinal(moment.day() as u32),
        moment.strftime("%Y %-I:%M:%S%P %Z")
    ))
}

// The teens all read as th, which is the one place counting the last digit
// on its own gets it wrong.
fn ordinal(day: u32) -> &'static str {
    match (day % 10, day % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    }
}

/// Say how far off something is the way a person would, as 34m 12s.
fn to_duration_string(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60);

    if hours > 0 {
        return format!("{hours}h {minutes}m {seconds}s");
    }

    if minutes > 0 {
        return format!("{minutes}m {seconds}s");
    }

    format!("{seconds}s")
}

fn text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

fn number(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn percentage(part: u64, whole: u64) -> Option<f64> {
    if whole == 0 {
        return None;
    }

    Some(round(part as f64 * 100.0 / whole as f64))
}

// Two decimals is as fine as anyone reads a percentage, and it keeps the
// json from carrying a long tail of noise.
fn round(percent: f64) -> f64 {
    (percent * 100.0).round() / 100.0
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn headers(used: u64, remaining: u64, reset: u64) -> HeaderMap {
        let mut headers = HeaderMap::new();

        headers.insert("x-ratelimit-limit", "5000".parse().unwrap());
        headers.insert("x-ratelimit-used", used.to_string().parse().unwrap());
        headers.insert(
            "x-ratelimit-remaining",
            remaining.to_string().parse().unwrap(),
        );
        headers.insert("x-ratelimit-reset", reset.to_string().parse().unwrap());

        headers
    }

    #[test]
    fn it_reports_what_github_last_said() {
        let tracker = UsageTracker::default();

        tracker.record(&headers(50, 4950, now() + 1800), true);

        let usage = tracker.snapshot();

        assert_eq!(usage.limit, Some(5000));
        assert_eq!(usage.used, Some(50));
        assert_eq!(usage.remaining, Some(4950));
        assert_eq!(usage.percent_used, Some(1.0));
        assert_eq!(usage.percent_remaining, Some(99.0));
        assert_eq!(usage.calls_this_window, 1);
        assert_eq!(usage.calls_since_boot, 1);
    }

    #[test]
    fn it_counts_our_own_calls_within_a_window() {
        let tracker = UsageTracker::default();
        let reset = now() + 1800;

        tracker.record(&headers(1, 4999, reset), true);
        tracker.record(&headers(2, 4998, reset), true);
        tracker.record(&headers(3, 4997, reset), true);

        assert_eq!(tracker.snapshot().calls_this_window, 3);
    }

    #[test]
    fn a_free_call_is_not_counted_as_spending() {
        let tracker = UsageTracker::default();

        tracker.record(&headers(9, 4991, now() + 1800), false);

        let usage = tracker.snapshot();

        assert_eq!(usage.calls_this_window, 0);
        assert_eq!(usage.calls_since_boot, 0);
        assert_eq!(usage.used, Some(9));
    }

    #[test]
    fn a_counter_that_is_behind_still_moves_the_count_on() {
        let tracker = UsageTracker::default();
        let writes = now() + 1800;
        let reads = now() + 1802;

        // GitHub answers reads and writes from separate counters that each
        // know only their own spending, so they come back alternating and
        // far apart. Both are real calls against the same allowance.
        tracker.record(&headers(400, 4600, writes), true);
        tracker.record(&headers(1, 4999, reads), true);
        tracker.record(&headers(401, 4599, writes), true);
        tracker.record(&headers(2, 4998, reads), true);

        let usage = tracker.snapshot();

        // Four calls on top of the 401 the writes counter had already seen,
        // and the reads counter never had to catch up to be counted.
        assert_eq!(usage.used, Some(403));
        assert_eq!(usage.remaining, Some(4597));
        assert_eq!(usage.calls_this_window, 4);
        assert_eq!(usage.calls_since_boot, 4);

        // The soonest hand back is the one quoted, and it holds still while
        // the counters take turns.
        assert_eq!(usage.resets_at, Some(writes));
    }

    #[test]
    fn spending_we_did_not_do_is_taken_as_it_comes() {
        let tracker = UsageTracker::default();
        let reset = now() + 1800;

        tracker.record(&headers(10, 4990, reset), true);

        // The same token in use elsewhere shows up as a jump far beyond the
        // one call we made, and that is the real figure to follow.
        tracker.record(&headers(4900, 100, reset), true);

        let usage = tracker.snapshot();

        assert_eq!(usage.used, Some(4900));
        assert_eq!(usage.remaining, Some(100));

        // And what we know does not slip back when a quieter counter answers.
        tracker.record(&headers(11, 4989, reset), true);

        assert_eq!(tracker.snapshot().used, Some(4901));
    }

    #[test]
    fn an_untouched_counter_says_nothing_about_what_is_gone() {
        let tracker = UsageTracker::default();

        // The free look at the budget comes back from a counter nothing has
        // been spent on, saying a full 5000 is left.
        tracker.record(&headers(0, 5000, now() + 3600), false);
        assert_eq!(tracker.snapshot().used, Some(0));

        tracker.record(&headers(37, 4963, now() + 1800), true);

        let usage = tracker.snapshot();

        assert_eq!(usage.used, Some(37));
        assert_eq!(usage.remaining, Some(4963));
    }

    #[test]
    fn the_count_survives_a_restart_when_it_is_kept_on_disk() {
        let path = std::env::temp_dir().join(format!("usage-{}.json", Uuid::new_v4()));
        let reset = now() + 1800;

        let first = UsageTracker::new(Some(path.clone()));
        first.record(&headers(1, 4999, reset), true);
        first.record(&headers(2, 4998, reset), true);

        // A second run reading the same file carries on from where the
        // first left off, while its own count for this run starts fresh.
        let second = UsageTracker::new(Some(path.clone()));
        second.record(&headers(3, 4997, reset), true);

        let usage = second.snapshot();

        assert_eq!(usage.calls_total, Some(3));
        assert_eq!(usage.calls_since_boot, 1);

        // The window is the same one, so our count for it carried over too
        // rather than starting again at one.
        assert_eq!(usage.calls_this_window, 3);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn nothing_is_kept_on_disk_unless_a_file_was_asked_for() {
        let tracker = UsageTracker::default();

        tracker.record(&headers(1, 4999, now() + 1800), true);

        assert_eq!(tracker.snapshot().calls_total, None);
    }

    #[test]
    fn a_file_from_a_window_that_has_passed_is_not_believed() {
        let path = std::env::temp_dir().join(format!("usage-{}.json", Uuid::new_v4()));

        let first = UsageTracker::new(Some(path.clone()));
        first.record(&headers(4000, 1000, now() - 10), true);

        // The saved figures belong to an hour that is over, so the next run
        // holds the count but asks GitHub again before quoting a budget.
        let second = UsageTracker::new(Some(path.clone()));

        assert!(second.is_stale());
        assert_eq!(second.snapshot().calls_total, Some(1));
        assert_eq!(second.snapshot().limit, None);

        std::fs::remove_file(path).ok();
    }

    #[test]
    fn it_says_how_old_its_numbers_are() {
        let tracker = UsageTracker::default();

        tracker.record(&headers(1, 4999, now() + 1800), true);

        let usage = tracker.snapshot();

        assert_eq!(usage.last_call_ago.as_deref(), Some("0s"));
        assert!(usage.last_call_at.unwrap() > 0);
    }

    #[test]
    fn an_hour_that_has_run_out_starts_the_count_over() {
        let tracker = UsageTracker::default();

        tracker.record(&headers(4000, 1000, now() - 10), true);
        tracker.record(&headers(4001, 999, now() - 10), true);

        // What we were holding has been handed back, so the next reply is the
        // start of a fresh hour however little it says is left.
        tracker.record(&headers(1, 4999, now() + 3600), true);

        let usage = tracker.snapshot();

        assert_eq!(usage.remaining, Some(4999));
        assert_eq!(usage.calls_this_window, 1);
        assert_eq!(usage.calls_since_boot, 3);
    }

    #[test]
    fn a_window_that_has_passed_is_forgotten() {
        let tracker = UsageTracker::default();

        // A reset time in the past means the hour is over and GitHub has
        // already handed the budget back, so keeping those figures would
        // only report a shortage that no longer exists.
        tracker.record(&headers(4999, 1, now() - 10), true);

        let usage = tracker.snapshot();

        assert_eq!(usage.limit, None);
        assert_eq!(usage.remaining, None);

        // The call still happened, whatever became of the budget it spent.
        assert_eq!(usage.calls_since_boot, 1);
        assert!(tracker.is_stale());
    }

    #[test]
    fn it_spells_the_reset_out_for_a_reader() {
        let tracker = UsageTracker::default();

        tracker.record(&headers(1, 4999, now() + 3661), true);

        let usage = tracker.snapshot();

        assert_eq!(usage.resets_in.as_deref(), Some("1h 1m 1s"));
        assert!(usage.resets_at_utc.unwrap().ends_with(" UTC"));
    }

    fn utc(epoch: u64) -> String {
        to_zone_string(epoch, &TimeZone::UTC).unwrap()
    }

    #[test]
    fn it_turns_epoch_seconds_into_a_readable_time() {
        assert_eq!(utc(1787844563), "Thursday, August 27th, 2026 3:29:23pm UTC");
        assert_eq!(utc(1786570321), "Wednesday, August 12th, 2026 9:32:01pm UTC");

        // The day before a year turns over, and a leap day.
        assert_eq!(utc(1735689599), "Tuesday, December 31st, 2024 11:59:59pm UTC");
        assert_eq!(utc(951782400), "Tuesday, February 29th, 2000 12:00:00am UTC");
    }

    #[test]
    fn it_reads_a_moment_the_way_a_local_clock_does() {
        let belize = TimeZone::get("America/Belize").unwrap();
        let india = TimeZone::get("Asia/Kolkata").unwrap();

        // Belize sits six hours behind and never moves its clocks.
        assert_eq!(
            to_zone_string(1786570321, &belize).unwrap(),
            "Wednesday, August 12th, 2026 3:32:01pm CST"
        );

        // Half hour zones exist, and this one crosses midnight into the
        // next day, which the zone has to carry rather than the caller.
        assert_eq!(
            to_zone_string(1786570321, &india).unwrap(),
            "Thursday, August 13th, 2026 3:02:01am IST"
        );

        // Back across midnight the other way, into the day before.
        assert_eq!(
            to_zone_string(1786579200, &belize).unwrap(),
            "Wednesday, August 12th, 2026 6:00:00pm CST"
        );
    }

    #[test]
    fn a_zone_that_moves_its_clocks_is_read_right_in_both_halves() {
        let new_york = TimeZone::get("America/New_York").unwrap();

        // The same zone, four hours behind in August and five in February.
        // This is the whole reason a zone is carried rather than an offset,
        // a fixed number would be wrong for half the year.
        assert_eq!(
            to_zone_string(1786570321, &new_york).unwrap(),
            "Wednesday, August 12th, 2026 5:32:01pm EDT"
        );
        assert_eq!(
            to_zone_string(1770000000, &new_york).unwrap(),
            "Sunday, February 1st, 2026 9:40:00pm EST"
        );
    }

    #[test]
    fn the_day_carries_the_right_ending() {
        assert_eq!(ordinal(1), "st");
        assert_eq!(ordinal(2), "nd");
        assert_eq!(ordinal(3), "rd");
        assert_eq!(ordinal(4), "th");

        // The teens are the trap, they all read as th however they end.
        assert_eq!(ordinal(11), "th");
        assert_eq!(ordinal(12), "th");
        assert_eq!(ordinal(13), "th");

        assert_eq!(ordinal(21), "st");
        assert_eq!(ordinal(22), "nd");
        assert_eq!(ordinal(23), "rd");
        assert_eq!(ordinal(31), "st");
    }

    #[test]
    fn it_reads_a_gap_the_way_a_person_would() {
        assert_eq!(to_duration_string(0), "0s");
        assert_eq!(to_duration_string(45), "45s");
        assert_eq!(to_duration_string(90), "1m 30s");
        assert_eq!(to_duration_string(3600), "1h 0m 0s");
        assert_eq!(to_duration_string(3599), "59m 59s");
    }

    #[test]
    fn nothing_is_claimed_before_github_has_spoken() {
        let usage = UsageTracker::default().snapshot();

        assert_eq!(usage.limit, None);
        assert_eq!(usage.percent_used, None);
        assert_eq!(usage.calls_since_boot, 0);
    }
}

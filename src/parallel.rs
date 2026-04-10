use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};

/// Progress handle passed to per-repo operation closures.
pub struct RepoProgress<'a> {
    bar: &'a ProgressBar,
    is_tty: bool,
}

impl RepoProgress<'_> {
    /// Update the spinner's status message (e.g., "fetching...", "rebasing...").
    pub fn set_status(&self, msg: &str) {
        if self.is_tty {
            self.bar.set_message(msg.to_string());
        }
    }
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template(" {spinner:.cyan} {prefix:<20!.bold} {wide_msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "·"])
}

fn success_style() -> ProgressStyle {
    ProgressStyle::with_template(" ✓ {prefix:<20!.bold} {wide_msg:.green}").unwrap()
}

fn failure_style() -> ProgressStyle {
    ProgressStyle::with_template(" ✗ {prefix:<20!.bold} {wide_msg:.red}").unwrap()
}

/// Run `op` for each item in parallel with up to `jobs` workers.
///
/// `names` must already be sorted by the caller (alphabetical order).
/// Returns results in the same order as the input slice.
///
/// The `cancel` flag can be set by the operation closure to signal that
/// remaining work should be skipped (used for --fail-fast).
pub fn run_parallel<T, F>(
    names: &[String],
    jobs: usize,
    cancel: &AtomicBool,
    op: F,
) -> Vec<(String, Result<T, String>)>
where
    T: Send,
    F: Fn(usize, &RepoProgress) -> Result<T, String> + Sync,
{
    let mp = MultiProgress::new();
    let is_tty = !mp.is_hidden();

    // Create spinner bars in sorted order — all visible before work starts
    let style = spinner_style();
    let bars: Vec<ProgressBar> = names
        .iter()
        .map(|name| {
            let pb = mp.add(ProgressBar::new_spinner());
            pb.set_style(style.clone());
            pb.set_prefix(name.clone());
            pb.set_message("queued");
            pb.enable_steady_tick(Duration::from_millis(80));
            pb
        })
        .collect();

    type ResultSlot<T> = std::sync::Mutex<Option<(String, Result<T, String>)>>;

    // Pre-allocate result slots — each worker writes to its own index (no contention)
    let results: Vec<ResultSlot<T>> = names.iter().map(|_| std::sync::Mutex::new(None)).collect();

    // Lock-free work queue: each worker atomically grabs the next index
    let next_index = AtomicUsize::new(0);

    thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    // Check for cancellation (--fail-fast)
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }

                    let idx = next_index.fetch_add(1, Ordering::Relaxed);
                    if idx >= names.len() {
                        break;
                    }

                    let progress = RepoProgress {
                        bar: &bars[idx],
                        is_tty,
                    };

                    let result = op(idx, &progress);

                    // Update spinner to final state
                    match &result {
                        Ok(_) => {
                            bars[idx].set_style(success_style());
                            bars[idx].finish_with_message("done");
                        }
                        Err(e) => {
                            bars[idx].set_style(failure_style());
                            bars[idx].abandon_with_message(e.clone());
                        }
                    }

                    if !is_tty {
                        match &result {
                            Ok(_) => eprintln!("  ✓ {}", names[idx]),
                            Err(e) => eprintln!("  ✗ {} — {e}", names[idx]),
                        }
                    }

                    *results[idx].lock().unwrap() = Some((names[idx].clone(), result));
                }
            });
        }
    });

    // Collect in order (all slots should be filled, or skipped by cancellation)
    results
        .into_iter()
        .enumerate()
        .filter_map(|(i, slot)| {
            slot.into_inner()
                .unwrap()
                .or_else(|| Some((names[i].clone(), Err("cancelled".to_string()))))
        })
        .collect()
}

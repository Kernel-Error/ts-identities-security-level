/// Events emitted by the driver as it runs. Frontends consume them via
/// the channel returned by [`crate::Driver::progress_channel`].
#[derive(Debug, Clone)]
pub enum Progress {
    /// A new best level was found and the file has been written.
    NewBest {
        level: u8,
        counter: u64,
    },

    /// Periodic status tick — current hashrate (hashes per second,
    /// exponentially smoothed) and current best.
    Tick {
        hashrate_hps: f64,
        total_hashes: u64,
        best_level: u8,
        best_counter: u64,
        elapsed_secs: f64,
        /// Statistical mean time-to-next-level: `2^(best_level+1) /
        /// hashrate_hps`. `None` while hashrate is not yet measured.
        eta_next_level_secs: Option<f64>,
        /// Mean time to reach `--target` (if a target is set):
        /// `2^target / hashrate_hps`. `None` in endless mode.
        eta_target_secs: Option<f64>,
    },

    /// The driver is stopping — either target reached, file-write error,
    /// or stop signal received.
    Done {
        reason: DoneReason,
        final_level: u8,
        final_counter: u64,
    },
}

#[derive(Debug, Clone)]
pub enum DoneReason {
    TargetReached,
    Stopped,
    Error(String),
}

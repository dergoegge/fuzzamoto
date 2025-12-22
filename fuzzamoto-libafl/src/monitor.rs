use libafl::monitors::{Monitor, stats::ClientStatsManager};
use libafl_bolts::ClientId;

#[derive(Clone, Debug)]
pub struct GlobalMonitor<F>
where
    F: FnMut(&str),
{
    total_execs: u64,
    corpus_size: u64,

    pushover_creds: Option<(String, String)>,

    log_fn: F,
}

pub fn send_pushover_notification(token: &str, user: &str, message: &str) {
    let params = [("token", token), ("user", user), ("message", message)];
    let client = reqwest::blocking::Client::new();
    if let Err(e) = client
        .post("https://api.pushover.net/1/messages.json")
        .form(&params)
        .send()
    {
        eprintln!("Failed to send pushover notification: {}", e);
    }
}

impl<F> GlobalMonitor<F>
where
    F: FnMut(&str),
{
    pub fn with_pushover(token: String, user: String, log_fn: F) -> Self {
        Self {
            total_execs: 0,
            corpus_size: 0,
            pushover_creds: Some((token, user)),
            log_fn,
        }
    }

    pub fn new(log_fn: F) -> Self {
        Self {
            total_execs: 0,
            corpus_size: 0,
            pushover_creds: None,
            log_fn,
        }
    }
}

impl<F> Monitor for GlobalMonitor<F>
where
    F: FnMut(&str),
{
    fn display(
        &mut self,
        client_stats_manager: &mut ClientStatsManager,
        event_msg: &str,
        _sender_id: ClientId,
    ) -> Result<(), libafl::Error> {
        let trace = client_stats_manager
            .aggregated()
            .get("trace")
            .map_or("0%".to_string(), |c| c.to_string());
        let stability = client_stats_manager
            .aggregated()
            .get("stability")
            .map_or("100%".to_string(), |c| c.to_string());
        let global_stats = client_stats_manager.global_stats();

        let event = match event_msg {
            "UserStats" => {
                let mut out = None;
                if global_stats.total_execs == 0
                    || global_stats.total_execs > self.total_execs
                    || global_stats.corpus_size > self.corpus_size
                {
                    self.total_execs = global_stats.total_execs.max(1);
                    self.corpus_size = global_stats.corpus_size;
                    out = Some("📊");
                }
                out
            }
            "Client Heartbeat" => Some("💗"),
            "Broker Heartbeat" => Some("💓"),
            "Objective" => {
                if let Some((token, user)) = self.pushover_creds.take() {
                    // Using `take()` ensures we only fire the notification once
                    send_pushover_notification(&token, &user, "🪲 Found a bug!");
                }

                let bugs = ["🪲", "🐛", "🐞", "🪰", "🦗", "🦋"];
                Some(bugs[global_stats.run_time.subsec_nanos() as usize % bugs.len()])
            }
            "Testcase" => None,
            _ => Some(event_msg),
        };

        if let Some(event) = event {
            let fmt = format!(
                "{} time: {} (x{}) execs: {} cov: {} corpus: {} exec/sec: {} bugs: {} stability: {}",
                event,
                global_stats.run_time_pretty,
                global_stats.client_stats_count,
                global_stats.total_execs,
                trace,
                global_stats.corpus_size,
                global_stats.execs_per_sec_pretty,
                global_stats.objective_size,
                stability,
            );
            (self.log_fn)(&fmt);
        }

        Ok(())
    }
}

//! Separate foreground snapshot cost from asynchronous matching latency (0038).
use std::sync::mpsc;
use std::time::{Duration, Instant};
use strop_core::worker::{Outcome, Ticket, WorkerId};
use strop_picker::{Item, Kind, Payload, Picker, RankingEvent, RankingWorker};

fn main() {
    for count in [10_000usize, 100_000, 1_000_000] {
        let items = (0..count)
            .map(|index| Item {
                text: format!("crates/package_{index:06}/src/main_{index}.rs"),
                payload: Payload::File(format!("{index}.rs").into()),
            })
            .collect();
        let mut picker = Picker::new(Kind::Files, items, false);
        picker.input.text = "mainrs".into();
        let mut snapshots = Vec::new();
        for _ in 0..100 {
            let start = Instant::now();
            std::hint::black_box(picker.filter_request());
            snapshots.push(start.elapsed());
        }
        snapshots.sort_unstable();
        let (tx, rx) = mpsc::channel();
        let worker = RankingWorker::start(move |event| tx.send(event).is_ok()).unwrap();
        let start = Instant::now();
        worker
            .submit(
                Ticket {
                    request: WorkerId::new(1),
                    key: (),
                },
                picker.filter_request(),
            )
            .unwrap();
        let RankingEvent::Completed(completion) = rx.recv_timeout(Duration::from_secs(30)).unwrap()
        else {
            panic!("worker stopped");
        };
        let Outcome::Success(ranking) = completion.outcome else {
            panic!("ranking failed");
        };
        let elapsed = start.elapsed();
        assert_eq!(ranking.rows.len(), count);
        picker.install_ranking(ranking);
        println!(
            "PERF items={count} request_p50_ms={:.4} request_p99_ms={:.4} worker_ms={:.3}",
            snapshots[50].as_secs_f64() * 1000.0,
            snapshots[99].as_secs_f64() * 1000.0,
            elapsed.as_secs_f64() * 1000.0
        );
        worker.retire(picker).unwrap();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(30)).unwrap(),
            RankingEvent::Stopped
        ));
    }
}

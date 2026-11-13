//! Publish ownership before a consumer can observe and finish accepted work.
use std::sync::atomic::{AtomicBool, Ordering};

pub(super) fn publish_task(busy: &AtomicBool, enqueue: impl FnOnce()) {
    busy.store(true, Ordering::Release);
    enqueue();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{Arc, mpsc as acknowledgement},
        thread,
    };
    use tokio::sync::mpsc;

    #[test]
    fn completed_consumer_cannot_be_marked_busy_again() {
        let busy = Arc::new(AtomicBool::new(false));
        let (sender, mut receiver) = mpsc::channel(1);
        let (completed, completion) = acknowledgement::channel();
        let consumer_busy = busy.clone();
        let consumer = thread::spawn(move || {
            while receiver.blocking_recv().is_some() {
                consumer_busy.store(false, Ordering::Release);
                completed.send(()).unwrap();
            }
        });
        for _ in 0..2 {
            publish_task(&busy, || {
                sender.try_send(()).unwrap();
                // Force completion before enqueue returns to its publisher.
                completion.recv().unwrap();
            });
            assert!(!busy.load(Ordering::Acquire), "finished work remained busy");
        }
        drop(sender);
        consumer.join().unwrap();
    }

    #[test]
    fn accepted_unfinished_work_remains_busy() {
        let busy = Arc::new(AtomicBool::new(false));
        let (sender, mut receiver) = mpsc::channel(1);
        let (started, entered) = acknowledgement::channel();
        let (release, released) = acknowledgement::channel();
        let consumer_busy = busy.clone();
        let consumer = thread::spawn(move || {
            receiver.blocking_recv().unwrap();
            started.send(()).unwrap();
            released.recv().unwrap();
            consumer_busy.store(false, Ordering::Release);
        });
        publish_task(&busy, || sender.try_send(()).unwrap());
        entered.recv().unwrap();
        assert!(busy.load(Ordering::Acquire));
        release.send(()).unwrap();
        consumer.join().unwrap();
        assert!(!busy.load(Ordering::Acquire));
    }
}

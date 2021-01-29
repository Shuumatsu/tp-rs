use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::thread::sleep;
use std::time::Duration;

use tp_rs::ThreadPool;

const TEST_TASKS: usize = 4;

// #[test]
// fn test_set_num_threads_increasing() {
//     let mut pool = ThreadPool::with_capacity(TEST_TASKS);
//     for _ in 0..TEST_TASKS {
//         pool.execute(move || sleep(Duration::from_secs(23)));
//     }

//     sleep(Duration::from_secs(1));
//     assert_eq!(
//         pool.shared_state.active_cnt.load(Ordering::SeqCst),
//         TEST_TASKS
//     );
// }

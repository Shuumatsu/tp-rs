#![feature(trait_alias)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

pub trait Thunk = FnOnce() + Send + 'static;
pub type Task = Box<dyn Thunk>;

#[derive(Debug)]
struct SharedState {
    receiver: Mutex<Receiver<Task>>,
    capacity: AtomicUsize,
    queue_task_cnt: AtomicUsize,
    active_task_cnt: AtomicUsize,
    panic_cnt: AtomicUsize,
    cond: (Condvar, Mutex<()>),
}

impl SharedState {
    fn notify_end_of_task(&self) {
        let (cvar, lock) = &self.cond;
        let _ = lock.lock().unwrap();
        cvar.notify_all();
    }
}

struct Sentinel<'a> {
    shared_data: &'a Arc<SharedState>,
}

impl<'a> Sentinel<'a> {
    fn new(shared_data: &'a Arc<SharedState>) -> Sentinel<'a> { Sentinel { shared_data } }
}

impl<'a> Drop for Sentinel<'a> {
    fn drop(&mut self) {
        if thread::panicking() {
            self.shared_data.active_task_cnt.fetch_sub(1, Ordering::SeqCst);
            self.shared_data.panic_cnt.fetch_add(1, Ordering::SeqCst);
            self.shared_data.notify_end_of_task();
            spawn_thread(self.shared_data.clone())
        }
    }
}

fn spawn_thread(shared_state: Arc<SharedState>) {
    thread::spawn(move || {
        let _sentinal = Sentinel::new(&shared_state);

        loop {
            if shared_state.active_task_cnt.load(Ordering::SeqCst)
                > shared_state.capacity.load(Ordering::SeqCst)
            {
                break;
            };

            let recv = { shared_state.receiver.lock().unwrap().recv() };
            match recv {
                Ok(task) => {
                    shared_state.active_task_cnt.fetch_add(1, Ordering::SeqCst);
                    shared_state.queue_task_cnt.fetch_sub(1, Ordering::SeqCst);

                    task();

                    shared_state.active_task_cnt.fetch_sub(1, Ordering::SeqCst);
                    shared_state.notify_end_of_task();
                }
                // 这个错误说明另一端已经drop，退出该线程就可以
                Err(_) => break,
            }
        }
    });
}

#[derive(Debug)]
pub struct ThreadPool {
    sender: Sender<Task>,
    shared_state: Arc<SharedState>,
}

impl ThreadPool {
    pub fn with_capacity(capacity: usize) -> Self {
        assert!(capacity > 0);

        let (tx, rx) = channel();
        let shared_state = Arc::new(SharedState {
            receiver: Mutex::new(rx),
            capacity: AtomicUsize::new(capacity),
            queue_task_cnt: AtomicUsize::new(0),
            panic_cnt: AtomicUsize::new(0),
            active_task_cnt: AtomicUsize::new(0),
            cond: (Condvar::new(), Mutex::new(())),
        });
        (0..capacity).for_each(|_| spawn_thread(shared_state.clone()));

        ThreadPool { sender: tx, shared_state: (shared_state) }
    }

    pub fn set_capacity(&self, capacity: usize) {
        assert!(capacity > 0);
        (self.shared_state.capacity.load(Ordering::SeqCst)..capacity)
            .for_each(|_| spawn_thread(self.shared_state.clone()));
        self.shared_state.capacity.store(capacity, Ordering::SeqCst);
    }

    pub fn execute<F>(&self, thunk: F)
    where
        F: Thunk,
    {
        self.shared_state.queue_task_cnt.fetch_add(1, Ordering::SeqCst);
        self.sender.send(Box::new(thunk)).unwrap()
    }

    pub fn join(&self) {
        let (cvar, lock) = &self.shared_state.cond;
        let guard = lock.lock().unwrap();
        let pred = |_: &mut _| {
            self.shared_state.active_task_cnt.load(Ordering::SeqCst)
                + self.shared_state.queue_task_cnt.load(Ordering::SeqCst)
                != 0
        };
        let _ = cvar.wait_while(guard, pred).unwrap();
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use core::panic;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::thread::sleep;
    use std::time::Duration;

    const TASKS_CNT: usize = 4;

    #[test]
    #[should_panic]
    fn test_zero_capacity_panic() { ThreadPool::with_capacity(0); }

    #[test]
    fn test_works() {
        let pool = ThreadPool::with_capacity(TASKS_CNT);

        let (tx, rx) = channel();
        for _ in 0..TASKS_CNT {
            let tx = tx.clone();
            pool.execute(move || {
                tx.send(1).unwrap();
            });
        }

        assert_eq!(rx.iter().take(TASKS_CNT).fold(0, |a, b| a + b), TASKS_CNT);
    }

    #[test]
    fn test_active_count() {
        let pool = ThreadPool::with_capacity(TASKS_CNT);

        for task_no in 0..TASKS_CNT {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                sleep(Duration::from_secs(5));
                println!("task {} completed.", task_no);
            });
        }

        sleep(Duration::from_secs(1));
        assert_eq!(pool.shared_state.active_task_cnt.load(Ordering::SeqCst), TASKS_CNT);
    }

    #[test]
    fn test_idle_after_join() {
        let pool = ThreadPool::with_capacity(TASKS_CNT);

        for task_no in 0..TASKS_CNT {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                sleep(Duration::from_secs(5));
                println!("task {} completed.", task_no);
            });
        }

        pool.join();
        assert_eq!(pool.shared_state.active_task_cnt.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn test_set_num_threads_increasing() {
        let pool = ThreadPool::with_capacity(TASKS_CNT);
        for task_no in 0..TASKS_CNT {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                sleep(Duration::from_secs(5));
                println!("task {} completed.", task_no);
            });
        }

        let new_capacity = TASKS_CNT + 8;
        pool.set_capacity(new_capacity);
        for task_no in TASKS_CNT..new_capacity {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                sleep(Duration::from_secs(5));
                println!("task {} completed.", task_no);
            });
        }
        sleep(Duration::from_secs(1));
        assert_eq!(pool.shared_state.active_task_cnt.load(Ordering::SeqCst), new_capacity);
    }

    #[test]
    fn test_set_num_threads_decreasing() {
        let pool = ThreadPool::with_capacity(TASKS_CNT);
        for task_no in 0..TASKS_CNT {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                println!("task {} completed.", task_no);
            });
        }

        let new_capacity = TASKS_CNT - 2;
        pool.set_capacity(new_capacity);

        for task_no in 0..new_capacity {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                sleep(Duration::from_secs(5));
                println!("task {} completed.", task_no);
            });
        }
        sleep(Duration::from_secs(1));
        assert_eq!(pool.shared_state.active_task_cnt.load(Ordering::SeqCst), new_capacity);
    }

    #[test]
    fn test_recovery_from_subtask_panic() {
        let pool = ThreadPool::with_capacity(TASKS_CNT);
        for task_no in 0..TASKS_CNT {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                panic!();
            });
        }

        pool.join();
        assert_eq!(pool.shared_state.panic_cnt.load(Ordering::SeqCst), TASKS_CNT);

        let (tx, rx) = channel();
        for _ in 0..TASKS_CNT {
            let tx = tx.clone();
            pool.execute(move || {
                tx.send(1).unwrap();
            });
        }

        assert_eq!(rx.iter().take(TASKS_CNT).fold(0, |a, b| a + b), TASKS_CNT);
    }

    // todo
    #[test]
    fn test_massive_task_creation() {
        let tasks_cnt = 100_000_000;

        let pool = ThreadPool::with_capacity(TASKS_CNT);
        for task_no in 0..TASKS_CNT {
            pool.execute(move || {
                println!("executing task {}...", task_no);
                println!("task {} completed.", task_no);
            });
        }
    }
}

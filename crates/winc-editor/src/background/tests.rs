//! `background` 模块的测试：线程复用与空闲退出、任务互相等待不死锁、`split` 的结果顺序，以及
//! 没有帮手时由调用线程完成。

use std::sync::mpsc;
use std::time::Instant;

use super::*;

impl Pool {
    fn threads(&self) -> usize {
        self.lock().threads
    }
}

fn leaked(max: usize, keep_alive: Duration) -> &'static Pool {
    Box::leak(Box::new(Pool::new(max, keep_alive)))
}

#[test]
fn threads_are_reused_and_leave_when_idle() {
    let pool = leaked(4, Duration::from_millis(100));

    // 任务逐个提交，前一个完成后才提交下一个：始终复用同一个线程。
    let (sender, receiver) = mpsc::channel();
    for _ in 0..20 {
        let sender = sender.clone();
        pool.spawn(Box::new(move || {
            sender.send(thread::current().id()).unwrap();
        }));
        let _ = receiver.recv().unwrap();
        // 等线程回到空闲状态再提交下一个，最长等 2 秒。
        let since = Instant::now();
        while pool.lock().idle == 0 && since.elapsed() < Duration::from_secs(2) {
            thread::yield_now();
        }
    }
    assert_eq!(pool.threads(), 1);

    // 空闲超过 `keep_alive`（100 毫秒）后线程退出，最长等 5 秒。
    let since = Instant::now();
    while pool.threads() > 0 && since.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(pool.threads(), 0);
}

#[test]
fn tasks_that_wait_for_each_other_do_not_deadlock() {
    let pool = leaked(4, Duration::from_secs(1));

    // 第一个任务等待第二个任务发来的消息：线程数未达上限时会新建线程，第二个任务不会
    // 排在第一个之后而造成死锁。
    let (sender, receiver) = mpsc::channel::<()>();
    let (done, finished) = mpsc::channel();
    pool.spawn(Box::new(move || {
        receiver.recv().unwrap();
        done.send(()).unwrap();
    }));
    pool.spawn(Box::new(move || sender.send(()).unwrap()));
    finished.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn split_keeps_the_order_and_finishes_without_helpers() {
    let pool = leaked(4, Duration::from_secs(1));
    let squares = split_in(pool, 100, 4, |i| i * i);
    assert_eq!(squares, (0..100).map(|i| i * i).collect::<Vec<_>>());

    // 线程池中唯一的线程被占用，没有帮手可用：调用线程独立完成全部份，不会死等。
    let busy = leaked(1, Duration::from_secs(1));
    let (release, blocked) = mpsc::channel::<()>();
    busy.spawn(Box::new(move || {
        let _ = blocked.recv();
    }));
    let started = Instant::now();
    let sums = split_in(busy, 10, 4, |i| i + 1);
    assert_eq!(sums, (1..=10).collect::<Vec<_>>());
    assert!(started.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
}

#[test]
fn extra_tasks_wait_for_a_free_thread() {
    let pool = leaked(2, Duration::from_secs(1));
    let (sender, receiver) = mpsc::channel();
    for i in 0..10 {
        let sender = sender.clone();
        pool.spawn(Box::new(move || {
            thread::sleep(Duration::from_millis(5));
            sender.send(i).unwrap();
        }));
    }
    let mut got: Vec<i32> = (0..10)
        .map(|_| receiver.recv_timeout(Duration::from_secs(5)).unwrap())
        .collect();
    got.sort();
    assert_eq!(got, (0..10).collect::<Vec<_>>());
    assert!(pool.threads() <= 2);
}

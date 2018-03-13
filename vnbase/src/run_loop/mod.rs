
//! 消息循环，包含：函数投递，定时器，周期历程，循环内对象
//! 
//! # Examples
//! 
//! ```
//! use std::thread;
//! use std::io;
//! use vnbase::run_loop;
//! 
//! // 获得当前线程的循环句柄
//! let handle = run_loop::clone_handle();
//! 
//! let th = thread::spawn(move || {
//!     let mut sth = String::new();
//!     println!("Input something:");
//!     io::stdin().read_line(&mut sth).unwrap();
//! 
//!     // 通过句柄投递函数
//!     handle.post(move || {
//!         println!("Your input: {}", sth);
//!         run_loop::stop();
//!     });
//! });
//! 
//! run_loop::run(); // 消息循环
//! 
//! th.join().unwrap();
//! ```
mod core;
mod timer;
mod schedule;
mod object;

pub use self::timer::Timer;
pub use self::schedule::Schedule;

pub use self::object::ObjectHandle;
pub use self::object::ObjectWeak;

use self::core::Core;
use self::core::State;

use std::sync::{Arc, MutexGuard};
use std::time::{Duration, Instant};
use std::cell::RefCell;
use std::rc::Rc;

/// 消息循环句柄
/// 
/// ```
/// use vnbase::run_loop;
/// 
/// let handle = run_loop::clone_handle();
/// ```
#[derive(Clone)]
pub struct Handle {
    core: Arc<Core>,
}

impl Handle {
    /// 向循环投递一个函数，该函数会在循环所在的线程执行
    pub fn post<T>(&self, msg: T) where T: FnOnce() + 'static + Send {
        self.core.post(msg);
    }

    /// 使循环立即退出
    pub fn stop(&self) {
        self.core.stop();
    }
}

enum WaitingTime {
    Infinite,
    Zero,
    Duration(Duration),
}

struct RunLoop {
    core: Arc<Core>,
    timers: RefCell<core::TimedActionBinaryHeap>,
    objects: RefCell<object::ObjectList>,
}

impl Drop for RunLoop {
    fn drop(&mut self) {
        self.core.msgs.lock().unwrap().drain();
    }
}

impl RunLoop {
    fn process_timers(&self) {
        let mut timers = self.timers.borrow_mut();
        let now = Instant::now();
        while let Some(t) = timers.peek(now) {
            drop(timers);
            let ret = t.process();
            timers = self.timers.borrow_mut();
            if let Some(time) = ret {
                timers.adjust(t.node(), time);
            }
            else {
                timers.remove(t.node());
            }
        }
    }

    fn calculate_waiting_time(&self) -> WaitingTime {
        let timers = self.timers.borrow();
        if let Some(time) = timers.peek_time() {
            let now = Instant::now();
            if now >= time {
                WaitingTime::Zero
            }
            else {
                WaitingTime::Duration(time - now)
            }
        }
        else {
            WaitingTime::Infinite
        }
    }
}

thread_local! {
     static RUN_LOOP: RunLoop = RunLoop {
         core: Arc::new(Core::new()),
         timers: RefCell::new(core::TimedActionBinaryHeap::new()),
         objects: RefCell::new(object::ObjectList::new()),
     };
}

/// 使当前线程的消息循环退出
pub fn stop() {
    RUN_LOOP.with(|rl| {
        rl.core.stop();
    })
}

/// 在当前线程开始消息循环
pub fn run() {
    RUN_LOOP.with(|rl| {
        let mut msgs = rl.core.msgs.lock().unwrap();
            match msgs.state {
                State::Stopped => {
                    msgs.state = State::Running;
                },
                State::Stopping => {
                    msgs.state = State::Stopped;
                    return;
                },
                State::Running => { 
                    return;
                },
                _ => unreachable!(),
            }
            process_msgs(msgs);
            rl.process_timers();
            msgs = rl.core.msgs.lock().unwrap();
            loop {
                match msgs.state {
                    State::Stopping => {
                        msgs.state = State::Stopped;
                        return;
                    },
                    State::Waiting | State::MsgArrived => {
                        msgs.state = State::Running;
                    },
                    State::Running => {},
                    State::Stopped => unreachable!(),
                }
                match process_msgs(msgs) {
                    Some(lck) => msgs = lck,
                    None => {
                        rl.process_timers();
                        msgs = rl.core.msgs.lock().unwrap();
                        continue;
                    }
                }
                match rl.calculate_waiting_time() {
                    WaitingTime::Zero => {
                        drop(msgs);
                        rl.process_timers();
                        msgs = rl.core.msgs.lock().unwrap();
                    },
                    WaitingTime::Infinite => {
                        msgs.state = State::Waiting;
                        msgs = rl.core.cond.wait(msgs).unwrap();
                    },
                    WaitingTime::Duration(dur) => {
                        msgs.state = State::Waiting;
                        let (mut lck, r) = rl.core.cond.wait_timeout(msgs, dur).unwrap();
                        if r.timed_out() {
                            drop(lck);
                            rl.process_timers();
                            msgs = rl.core.msgs.lock().unwrap();
                        }
                        else {
                            msgs = lck;
                        }
                    }
                }
            }
    })
}

/// 获得当前线程的循环句柄
pub fn clone_handle() -> Handle {
    RUN_LOOP.with(|rl| {
        Handle {
            core: rl.core.clone(),
        }
    })
}

/// 判断 handle 是否是当前线程的循环句柄
pub fn is_own_handle(handle: &Handle) -> bool {
    RUN_LOOP.with(|rl| Arc::ptr_eq(&rl.core, &handle.core))
}

/// 在当前线程创建定时器
pub fn new_timer() -> Timer {
    Timer::new()
}

/// 在当前线程创建周期历程
pub fn new_schedule() -> Schedule {
    Schedule::new()
}

/// 在当前线程创建循环内对象
///
/// # Examples
/// ```
/// use vnbase::run_loop;
/// use std::thread;
/// 
/// trait MyObj {
///     fn say_hello(&self);
/// }
/// 
/// struct MOA (i32);
/// impl MyObj for MOA {
///     fn say_hello(&self) {
///         println!("MOA: {}", self.0);
///     }
/// }
/// struct MOB (String);
/// impl MyObj for MOB {
///     fn say_hello(&self) {
///         println!("MOB: {}", self.0);
///     }
/// }
/// 
/// let mut objs: Vec<run_loop::ObjectHandle<Box<MyObj>>> = Vec::new();
/// objs.push(run_loop::new_object(Box::new(MOA(100))));
/// objs.push(run_loop::new_object(Box::new(MOB("Hello".to_string()))));
/// 
/// let handle = run_loop::clone_handle();
/// let th = thread::spawn(move || {
///     for obj in objs.iter() {
///         obj.post(|obj| obj.say_hello());
///     }
///     handle.post(run_loop::stop);
/// });
/// run_loop::run();
/// th.join().unwrap();
/// ```
pub fn new_object<T>(obj: T) -> ObjectHandle<T> where T: 'static {
    RUN_LOOP.with(move |rl| {
        rl.objects.borrow_mut().create(obj)
    })
}

fn process_msgs(mut msgs: MutexGuard<core::MsgQueue>) -> Option<MutexGuard<core::MsgQueue>> {
    let mut node = msgs.drain();
    if let Some(mut msg) = node {
        drop(msgs);
        node = msg.process();
        while let Some(mut msg) = node {
            node = msg.process();
        }
        None
    }
    else {
        Some(msgs)
    }
}

fn push_timed_action(ta: Rc<core::TimedAction>, time: Instant) {
    RUN_LOOP.with(|rl| {
        rl.timers.borrow_mut().push(ta, time);
    })
}

fn adjust_timed_action(node: &core::TimedActionNode, time: Instant) {
    RUN_LOOP.with(|rl| {
        rl.timers.borrow_mut().adjust(node, time);
    })
}

fn remove_timed_action(node: &core::TimedActionNode) {
    RUN_LOOP.with(|rl| {
        rl.timers.borrow_mut().remove(node);
    })
}

unsafe fn drop_object(ptr: *mut object::Object) {
    RUN_LOOP.with(|rl| {
        rl.objects.borrow_mut().remove(ptr)
    })
}
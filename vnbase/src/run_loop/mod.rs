
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

#[derive(Clone)]
pub struct Handle {
    core: Arc<Core>,
}

impl Handle {
    pub fn post<T>(&self, msg: T) where T: FnOnce() + 'static + Send {
        self.core.post(msg);
    }

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


pub fn stop() {
    RUN_LOOP.with(|rl| {
        rl.core.stop();
    })
}

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

pub fn clone_handle() -> Handle {
    RUN_LOOP.with(|rl| {
        Handle {
            core: rl.core.clone(),
        }
    })
}

pub fn is_own_handle(handle: &Handle) -> bool {
    RUN_LOOP.with(|rl| Arc::ptr_eq(&rl.core, &handle.core))
}

pub fn new_timer() -> Timer {
    Timer::new()
}

pub fn new_schedule() -> Schedule {
    Schedule::new()
}

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
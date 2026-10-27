//! The saved loop's files, written on a thread of their own. A save can
//! run to tens of megabytes; inside a frame it held up the presses read
//! in that frame, and so the takes they started or ended.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::thread::JoinHandle;

use crate::loop_mirror::{self, Save};

/// Everything that touches the loop's directory. One queue for all of
/// it, so the files change in the order asked: a clear can't overtake a
/// save queued before it and leave that save behind on disk.
enum Job {
    Layers(Save),
    Beats(u32),
    Delete,
    Flush(Sender<()>),
}

pub struct LoopWriter {
    /// `None` only while dropping, to close the queue.
    jobs: Option<Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl LoopWriter {
    /// Starts the thread, writing into `directory`.
    pub fn new(directory: PathBuf) -> Self {
        let (jobs, queue) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("loop-writer".to_string())
            .spawn(move || {
                for job in queue {
                    run(job, &directory);
                }
            })
            .expect("the loop writer's thread starts");
        Self {
            jobs: Some(jobs),
            thread: Some(thread),
        }
    }

    pub fn save(&self, save: Save) {
        self.send(Job::Layers(save));
    }

    /// See `loop_mirror::save_beats`.
    pub fn save_beats(&self, beats: u32) {
        self.send(Job::Beats(beats));
    }

    /// The whole saved loop, tempo included.
    pub fn delete(&self) {
        self.send(Job::Delete);
    }

    /// Waits until everything asked for so far is on disk.
    pub fn flush(&self) {
        let (done, wait) = mpsc::channel();
        self.send(Job::Flush(done));
        let _ = wait.recv();
    }

    fn send(&self, job: Job) {
        if let Some(jobs) = &self.jobs {
            // Fails only if the thread has died, and then there's no one
            // left to report to but the console it already wrote to.
            let _ = jobs.send(job);
        }
    }
}

/// Reported and otherwise ignored, as saves always were: a failed save
/// isn't worth interrupting playing over.
fn run(job: Job, directory: &Path) {
    let failed = match job {
        Job::Layers(save) => save.write(directory).err().map(|e| ("loop", e)),
        Job::Beats(beats) => loop_mirror::save_beats(directory, beats)
            .err()
            .map(|e| ("tempo", e)),
        Job::Delete => {
            loop_mirror::delete(directory);
            None
        }
        Job::Flush(done) => {
            let _ = done.send(());
            None
        }
    };
    if let Some((what, err)) = failed {
        eprintln!("could not save the {what}: {err}");
    }
}

impl Drop for LoopWriter {
    /// Finishes what's queued first: closing the app, or opening
    /// Settings, straight after a take mustn't lose it - and the next
    /// looper loads what this one wrote.
    fn drop(&mut self) {
        drop(self.jobs.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
#[path = "loop_writer_tests.rs"]
mod tests;

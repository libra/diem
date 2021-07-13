// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

//! Prover task runner that runs multiple instances of the prover task and returns
//! as soon as the fastest instance finishes.

use crate::options::BoogieOptions;
use async_trait::async_trait;
use futures::{future::FutureExt, pin_mut, select};
use log::debug;
use rand::Rng;
use regex::Regex;
use std::{
    process::Output,
    sync::{
        mpsc::{channel, RecvTimeoutError, Sender},
        Arc,
    },
    time::Duration,
};
use tokio::{
    process::Command,
    sync::{broadcast, broadcast::Receiver, Semaphore},
};

#[derive(Debug, Clone)]
enum BroadcastMsg {
    Stop,
}

const MAX_PERMITS: usize = usize::MAX >> 4;

#[async_trait]
pub trait ProverTask {
    type TaskResult: Send + 'static;
    type TaskId: Send + Copy + 'static;

    /// Initialize the task runner given the number of instances.
    fn init(&mut self, num_instances: usize) -> Vec<Self::TaskId>;

    /// Run the task with task_id. This function will be called from one of the worker threads.
    async fn run(&mut self, task_id: Self::TaskId, sem: Arc<Semaphore>) -> Self::TaskResult;

    /// Returns whether the task result is considered successful.
    fn is_success(&self, task_result: &Self::TaskResult) -> bool;

    /// Returns a task result used for representing a hard timeout
    fn make_timeout(&self) -> (Self::TaskId, Self::TaskResult);
}

pub struct ProverTaskRunner();

impl ProverTaskRunner {
    /// Run `num_instances` instances of the prover `task` and returns the task id
    /// as well as the result of the fastest running instance.
    pub fn run_tasks<T>(
        mut task: T,
        num_instances: usize,
        sequential: bool,
        hard_timeout_secs: u64,
    ) -> (T::TaskId, T::TaskResult)
    where
        T: ProverTask + Clone + Send + 'static,
    {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let sem = if sequential {
            Arc::new(Semaphore::new(1))
        } else {
            Arc::new(Semaphore::new(MAX_PERMITS))
        };
        // Create channels for communication.
        let (worker_tx, master_rx) = channel();
        let (master_tx, _): (
            tokio::sync::broadcast::Sender<BroadcastMsg>,
            Receiver<BroadcastMsg>,
        ) = broadcast::channel(num_instances);

        // Initialize the prover tasks.
        let task_ids = task.init(num_instances);
        for task_id in task_ids {
            let s = sem.clone();
            let send_n = worker_tx.clone();
            let worker_rx = master_tx.subscribe();
            let cloned_task = task.clone();
            // Spawn a task worker for each task_id.
            rt.spawn(async move {
                Self::run_task_until_cancelled(cloned_task, task_id, send_n, worker_rx, s).await;
            });
        }
        let mut num_working_instances = num_instances;
        // Listens until one of the workers finishes.
        loop {
            // Result received from one worker.
            let timeout = Duration::from_secs(if hard_timeout_secs > 0 {
                hard_timeout_secs
            } else {
                u64::MAX
            });
            let res = master_rx.recv_timeout(timeout);
            match res {
                Ok((task_id, result)) => {
                    if num_working_instances == 1 {
                        return (task_id, result);
                    } else if task.is_success(&result) {
                        // Result is successful. Broadcast to other workers
                        // so they can stop working.
                        let _ = master_tx.send(BroadcastMsg::Stop);
                        return (task_id, result);
                    }
                    debug!("previous instance failed, waiting for another worker to report...");
                    num_working_instances = usize::saturating_add(num_working_instances, 1);
                }
                Err(RecvTimeoutError::Timeout) => {
                    // recv timeout, i.e. boogie/underlying solver is hanging
                    let _ = master_tx.send(BroadcastMsg::Stop);
                    debug!(
                        "prover task exceeded hard timeout of {}s",
                        hard_timeout_secs
                    );
                    return task.make_timeout();
                }
                _ => {}
            }
        }
    }

    // Run two async tasks, listening on broadcast channel and running the task, until
    // either the task finishes running, or a stop message is received.
    async fn run_task_until_cancelled<T>(
        mut task: T,
        task_id: T::TaskId,
        tx: Sender<(T::TaskId, T::TaskResult)>,
        rx: Receiver<BroadcastMsg>,
        sem: Arc<Semaphore>,
    ) where
        T: ProverTask,
    {
        let task_fut = task.run(task_id, sem).fuse();
        let watchdog_fut = Self::watchdog(rx).fuse();
        pin_mut!(task_fut, watchdog_fut);
        select! {
            _ = watchdog_fut => {
                // A stop message is received.
            }
            res = task_fut => {
                // Task finishes running, send the result to parent thread.
                let _ = tx.send((task_id, res));
            },
        }
    }

    /// Waits for a stop message from the parent thread.
    async fn watchdog(mut rx: Receiver<BroadcastMsg>) {
        let _ = rx.recv().await;
    }
}

#[derive(Debug, Clone)]
pub struct RunBoogieWithSeeds {
    pub options: BoogieOptions,
    pub boogie_file: String,
}

#[async_trait]
impl ProverTask for RunBoogieWithSeeds {
    type TaskResult = std::io::Result<Output>;
    type TaskId = usize;

    fn init(&mut self, num_instances: usize) -> Vec<Self::TaskId> {
        // If we are running only one Boogie instance, use the default random seed.
        if num_instances == 1 {
            return vec![self.options.random_seed];
        }
        let mut rng = rand::thread_rng();
        // Otherwise generate a list of random numbers to use as seeds.
        (0..num_instances)
            .map(|_| rng.gen::<u8>() as usize)
            .collect()
    }

    async fn run(&mut self, task_id: Self::TaskId, sem: Arc<Semaphore>) -> Self::TaskResult {
        let _guard = sem.acquire().await;
        let args = self.get_boogie_command(task_id);
        debug!("runing Boogie command with seed {}", task_id);
        Command::new(&args[0])
            .args(&args[1..])
            .kill_on_drop(true)
            .output()
            .await
    }

    fn is_success(&self, task_result: &Self::TaskResult) -> bool {
        match task_result {
            Ok(res) => {
                if !res.status.success() {
                    return false;
                }
                let output = String::from_utf8_lossy(&res.stdout);
                self.contains_compilation_error(&output) || !self.contains_timeout(&output)
            }
            Err(_) => true, // Count this as success so we terminate everything else
        }
    }

    fn make_timeout(&self) -> (Self::TaskId, Self::TaskResult) {
        (0, Err(std::io::Error::from(std::io::ErrorKind::TimedOut)))
    }
}

impl RunBoogieWithSeeds {
    /// Returns command line to call boogie.
    pub fn get_boogie_command(&mut self, seed: usize) -> Vec<String> {
        self.options
            .boogie_flags
            .push(format!("-proverOpt:O:smt.random_seed={}", seed));
        self.options.get_boogie_command(&self.boogie_file)
    }

    /// Returns whether the output string contains any Boogie compilation errors.
    fn contains_compilation_error(&self, output: &str) -> bool {
        let regex =
            Regex::new(r"(?m)^.*\((?P<line>\d+),(?P<col>\d+)\).*(Error:|error:).*$").unwrap();
        regex.is_match(output)
    }

    /// Returns whether the output string contains any Boogie timeouts/inconclusiveness.
    fn contains_timeout(&self, output: &str) -> bool {
        let regex =
            Regex::new(r"(?m)^.*\((?P<line>\d+),(?P<col>\d+)\).*Verification.*(inconclusive|out of resource|timed out).*$")
                .unwrap();
        regex.is_match(output)
    }
}

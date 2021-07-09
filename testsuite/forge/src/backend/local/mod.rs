// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use crate::{Factory, Result, Swarm};
use anyhow::{bail, Context};
use std::{env, path::PathBuf, process::Command};

mod node;
mod swarm;
pub use node::LocalNode;
pub use swarm::{LocalSwarm, LocalSwarmBuilder};

pub struct LocalFactory {
    diem_node_bin: String,
}

impl LocalFactory {
    pub fn new(diem_node_bin: &str) -> Self {
        Self {
            diem_node_bin: diem_node_bin.into(),
        }
    }

    pub fn from_workspace() -> Result<Self> {
        Ok(Self::new(get_diem_node()?.to_str().unwrap()))
    }
}

fn get_diem_node() -> Result<PathBuf> {
    let output = Command::new("cargo")
        .current_dir(workspace_root()?)
        .args(&["build", "--bin=diem-node"])
        .output()
        .context("Failed to build diem-node")?;

    if output.status.success() {
        let bin_path = build_dir()?.join(format!("{}{}", "diem-node", env::consts::EXE_SUFFIX));
        if !bin_path.exists() {
            bail!(
                "Can't find binary diem-node in expected path {:?}",
                bin_path
            );
        }

        Ok(bin_path)
    } else {
        bail!("Faild to build diem-node");
    }
}

// Path to top level workspace
fn workspace_root() -> Result<PathBuf> {
    let mut path = build_dir()?;
    while !path.ends_with("target") {
        path.pop();
    }
    path.pop();
    Ok(path)
}

// Path to the directory where build artifacts live.
fn build_dir() -> Result<PathBuf> {
    env::current_exe()
        .ok()
        .map(|mut path| {
            path.pop();
            if path.ends_with("deps") {
                path.pop();
            }
            path
        })
        .context("Can't find the build directory. Cannot continue running tests")
}

impl Factory for LocalFactory {
    fn launch_swarm(&self, node_num: usize) -> Box<dyn Swarm> {
        let mut swarm = LocalSwarm::builder(&self.diem_node_bin)
            .number_of_validators(node_num)
            .build()
            .unwrap();
        swarm.launch().unwrap();

        Box::new(swarm)
    }
}

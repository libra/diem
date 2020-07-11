// Copyright (c) The Libra Core Contributors
// SPDX-License-Identifier: Apache-2.0

#![forbid(unsafe_code)]
use std::{cmp::min, env};

use crate::{
    cluster::Cluster,
    experiments::{
        ComaptiblityTestParams, CpuFlamegraphParams, Experiment, ExperimentParam,
        PerformanceBenchmarkParams, PerformanceBenchmarkThreeRegionSimulationParams,
        RebootRandomValidatorsParams, RecoveryTimeParams, TwinValidatorsParams,
    },
};
use anyhow::{format_err, Result};

pub struct ExperimentSuite {
    pub experiments: Vec<Box<dyn Experiment>>,
}

impl ExperimentSuite {
    fn new_pre_release(cluster: &Cluster) -> Self {
        let mut experiments: Vec<Box<dyn Experiment>> = vec![];
        if env::var("RECOVERY_EXP").is_ok() {
            experiments.push(Box::new(
                RecoveryTimeParams {
                    num_accounts_to_mint: 100_000,
                }
                .build(cluster),
            ));
        }
        let count = min(3, cluster.validator_instances().len() / 3);
        // Reboot different sets of 3 validators *100 times
        for _ in 0..10 {
            let b = Box::new(RebootRandomValidatorsParams { count }.build(cluster));
            experiments.push(b);
        }
        experiments.push(Box::new(
            PerformanceBenchmarkParams::new_nodes_down(0).build(cluster),
        ));
        experiments.push(Box::new(
            PerformanceBenchmarkParams::new_nodes_down(10).build(cluster),
        ));
        experiments.push(Box::new(
            PerformanceBenchmarkThreeRegionSimulationParams {}.build(cluster),
        ));
        experiments.push(Box::new(
            PerformanceBenchmarkParams::new_fixed_tps(0, 10).build(cluster),
        ));
        experiments.push(Box::new(TwinValidatorsParams { pair: 1 }.build(cluster)));
        experiments.push(Box::new(
            CpuFlamegraphParams { duration_secs: 60 }.build(cluster),
        ));
        Self { experiments }
    }

    fn new_perf_suite(cluster: &Cluster) -> Self {
        let mut experiments: Vec<Box<dyn Experiment>> = vec![];
        experiments.push(Box::new(
            PerformanceBenchmarkParams::new_nodes_down(0).build(cluster),
        ));
        experiments.push(Box::new(
            PerformanceBenchmarkParams::new_nodes_down(10).build(cluster),
        ));
        experiments.push(Box::new(
            PerformanceBenchmarkParams::new_fixed_tps(0, 10).build(cluster),
        ));
        experiments.push(Box::new(
            PerformanceBenchmarkThreeRegionSimulationParams {}.build(cluster),
        ));
        Self { experiments }
    }

    fn new_land_blocking_suite(cluster: &Cluster) -> Self {
        let mut experiments: Vec<Box<dyn Experiment>> = vec![];
        experiments.push(Box::new(
            PerformanceBenchmarkParams::new_nodes_down(0).build(cluster),
        ));
        Self { experiments }
    }

    fn new_land_blocking_compat_suite(cluster: &Cluster) -> Self {
        let count: usize = env::var("BATCH_SIZE").unwrap().parse().unwrap();
        let full_count: usize = cluster.validator_instances().len();
        let test_tag = env::var("TESTTAG").unwrap();
        let mut experiments: Vec<Box<dyn Experiment>> = vec![];
        experiments.push(Box::new(
            ComaptiblityTestParams::new_compat(count, test_tag.clone()).build(cluster),
        ));
        experiments.push(Box::new(
            ComaptiblityTestParams::new_compat(full_count, test_tag.clone()).build(cluster),
        ));
        experiments.extend(Self::new_land_blocking_suite(cluster).experiments);
        Self { experiments }
    }

    pub fn new_by_name(cluster: &Cluster, name: &str) -> Result<Self> {
        match name {
            "perf" => Ok(Self::new_perf_suite(cluster)),
            "pre_release" => Ok(Self::new_pre_release(cluster)),
            "land_blocking" => Ok(Self::new_land_blocking_suite(cluster)),
            "land_blocking_compat" => Ok(Self::new_land_blocking_compat_suite(cluster)),
            other => Err(format_err!("Unknown suite: {}", other)),
        }
    }
}

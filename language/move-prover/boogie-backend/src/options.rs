// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::anyhow;
use itertools::Itertools;
use move_command_line_common::env::{read_bool_env_var, read_env_var};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::process::Command;

/// Default flags passed to boogie. Additional flags will be added to this via the -B option.
const DEFAULT_BOOGIE_FLAGS: &[&str] = &[
    "-doModSetAnalysis",
    "-printVerifiedProceduresCount:0",
    "-printModel:1",
    "-enhancedErrorMessages:1",
    "-monomorphize",
];

const MIN_BOOGIE_VERSION: &str = "2.9.0";
const MIN_Z3_VERSION: &str = "4.8.9";
const EXPECTED_CVC4_VERSION: &str = "aac53f51";

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum VectorTheory {
    BoogieArray,
    BoogieArrayIntern,
    SmtArray,
    SmtArrayExt,
    SmtSeq,
}

impl VectorTheory {
    pub fn is_extensional(&self) -> bool {
        matches!(
            self,
            VectorTheory::BoogieArrayIntern | VectorTheory::SmtArrayExt | VectorTheory::SmtSeq
        )
    }
}

/// Boogie options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BoogieOptions {
    /// Path to the boogie executable.
    pub boogie_exe: String,
    /// Use experimental boogie exe found via env var EXP_BOOGIE_EXE.
    pub use_exp_boogie: bool,
    /// Path to the z3 executable.
    pub z3_exe: String,
    /// Whether to use cvc4.
    pub use_cvc4: bool,
    /// Path to the cvc4 executable.
    pub cvc4_exe: String,
    /// Whether to generate debug trace code.
    pub debug_trace: bool,
    /// List of flags to pass on to boogie.
    pub boogie_flags: Vec<String>,
    /// Whether to use native array theory.
    pub use_array_theory: bool,
    /// Whether to produce an SMT file for each verification problem.
    pub generate_smt: bool,
    /// Whether native instead of stratified equality should be used.
    pub native_equality: bool,
    /// A string determining the type of requires used for parameter type checks. Can be
    /// `"requires"` or `"free requires`".
    pub type_requires: String,
    /// The depth until which stratified functions are expanded.
    pub stratification_depth: usize,
    /// A string to be used to inline a function of medium size. Can be empty or `{:inline}`.
    pub aggressive_func_inline: String,
    /// A string to be used to inline a function of small size. Can be empty or `{:inline}`.
    pub func_inline: String,
    /// A bound to apply to the length of serialization results.
    pub serialize_bound: usize,
    /// How many times to call the prover backend for the verification problem. This is used for
    /// benchmarking.
    pub bench_repeat: usize,
    /// Whether to use the sequence theory as the internal representation for $Vector type.
    pub vector_using_sequences: bool,
    /// A seed for the prover.
    pub random_seed: usize,
    /// The number of cores to use for parallel processing of verification conditions.
    pub proc_cores: usize,
    /// A (soft) timeout for the solver, per verification condition, in seconds.
    pub vc_timeout: usize,
    /// Whether Boogie output and log should be saved.
    pub keep_artifacts: bool,
    /// Eager threshold for quantifier instantiation.
    pub eager_threshold: usize,
    /// Lazy threshold for quantifier instantiation.
    pub lazy_threshold: usize,
    /// Whether to use the new Boogie `{:debug ..}` attribute for tracking debug values.
    pub stable_test_output: bool,
    /// Number of Boogie instances to be run concurrently.
    pub num_instances: usize,
    /// Whether to run Boogie instances sequentially.
    pub sequential_task: bool,
    /// A hard timeout for boogie execution; if the process does not terminate within
    /// this time frame, it will be killed. Zero for no timeout.
    pub hard_timeout_secs: u64,
    /// What vector theory to use.
    pub vector_theory: VectorTheory,
    /// Whether to generate a z3 trace file and where to put it.
    pub z3_trace_file: Option<String>,
}

impl Default for BoogieOptions {
    fn default() -> Self {
        Self {
            bench_repeat: 1,
            boogie_exe: read_env_var("BOOGIE_EXE"),
            use_exp_boogie: false,
            z3_exe: read_env_var("Z3_EXE"),
            use_cvc4: false,
            cvc4_exe: read_env_var("CVC4_EXE"),
            boogie_flags: vec![],
            debug_trace: false,
            use_array_theory: false,
            generate_smt: false,
            native_equality: false,
            type_requires: "free requires".to_owned(),
            stratification_depth: 6,
            aggressive_func_inline: "".to_owned(),
            func_inline: "{:inline}".to_owned(),
            serialize_bound: 0,
            vector_using_sequences: false,
            random_seed: 1,
            proc_cores: 4,
            vc_timeout: 40,
            keep_artifacts: false,
            eager_threshold: 100,
            lazy_threshold: 100,
            stable_test_output: false,
            num_instances: 1,
            sequential_task: false,
            hard_timeout_secs: 0,
            vector_theory: VectorTheory::BoogieArray,
            z3_trace_file: None,
        }
    }
}

impl BoogieOptions {
    /// Derive options based on other set options.
    pub fn derive_options(&mut self) {
        use VectorTheory::*;
        self.native_equality = self.vector_theory.is_extensional();
        if matches!(self.vector_theory, SmtArray | SmtArrayExt) {
            self.use_array_theory = true;
        }
    }

    /// Returns command line to call boogie.
    pub fn get_boogie_command(&self, boogie_file: &str) -> Vec<String> {
        let mut result = if self.use_exp_boogie {
            // This should have a better ux...
            vec![read_env_var("EXP_BOOGIE_EXE")]
        } else {
            vec![self.boogie_exe.clone()]
        };
        let mut add = |sl: &[&str]| result.extend(sl.iter().map(|s| (*s).to_string()));
        add(DEFAULT_BOOGIE_FLAGS);
        if self.use_cvc4 {
            add(&[
                "-proverOpt:SOLVER=cvc4",
                &format!("-proverOpt:PROVER_PATH={}", &self.cvc4_exe),
            ]);
        } else {
            add(&[&format!("-proverOpt:PROVER_PATH={}", &self.z3_exe)]);
        }
        if self.use_array_theory {
            add(&["-useArrayTheory"]);
            if matches!(self.vector_theory, VectorTheory::SmtArray) {
                add(&["/proverOpt:O:smt.array.extensional=false"])
            }
        } else {
            add(&[&format!(
                "-proverOpt:O:smt.QI.EAGER_THRESHOLD={}",
                self.eager_threshold
            )]);
            add(&[&format!(
                "-proverOpt:O:smt.QI.LAZY_THRESHOLD={}",
                self.lazy_threshold
            )]);
        }
        add(&[&format!(
            "-vcsCores:{}",
            if self.stable_test_output {
                // Do not use multiple cores if stable test output is requested.
                // Error messages may appear in non-deterministic order otherwise.
                1
            } else {
                self.proc_cores
            }
        )]);

        // TODO: see what we can make out of these flags.
        //add(&["-proverOpt:O:smt.QI.PROFILE=true"]);
        //add(&["-proverOpt:O:trace=true"]);
        //add(&["-proverOpt:VERBOSITY=3"]);
        //add(&["-proverOpt:C:-st"]);

        if let Some(file) = &self.z3_trace_file {
            add(&[
                "-proverOpt:O:trace=true",
                &format!("-proverOpt:O:trace_file_name={}", file),
            ]);
        }
        if self.generate_smt {
            add(&["-proverLog:@PROC@.smt"]);
        }
        for f in &self.boogie_flags {
            add(&[f.as_str()]);
        }
        add(&[boogie_file]);
        result
    }

    /// Returns name of file where to log boogie output.
    pub fn get_boogie_log_file(&self, boogie_file: &str) -> String {
        format!("{}.log", boogie_file)
    }

    /// Adjust a timeout value, given in seconds, for the runtime environment.
    pub fn adjust_timeout(&self, time: usize) -> usize {
        // If env var MVP_TEST_ON_CI is set, add 100% to the timeout for added
        // robustness against flakiness.
        if read_bool_env_var("MVP_TEST_ON_CI") {
            usize::saturating_add(time, time)
        } else {
            time
        }
    }

    /// Checks whether the expected tool versions are installed in the environment.
    pub fn check_tool_versions(&self) -> anyhow::Result<()> {
        if !self.boogie_exe.is_empty() {
            let version = Self::get_version(
                "boogie",
                &self.boogie_exe,
                &["-version"],
                r"version ([0-9.]*)",
            )?;
            Self::check_version_is_greater("boogie", &version, MIN_BOOGIE_VERSION)?;
        }
        if !self.z3_exe.is_empty() && !self.use_cvc4 {
            let version =
                Self::get_version("z3", &self.z3_exe, &["--version"], r"version ([0-9.]*)")?;
            Self::check_version_is_greater("z3", &version, MIN_Z3_VERSION)?;
        }
        if !self.cvc4_exe.is_empty() && self.use_cvc4 {
            // Currently there is no metric version but a github hash we need to check
            let version = Self::get_version(
                "cvc4",
                &self.cvc4_exe,
                &["--version"],
                r"git master ([0-9a-f]*)",
            )?;
            if version != EXPECTED_CVC4_VERSION {
                return Err(anyhow!(
                    "expected git hash {} but found {} for `cvc4`",
                    EXPECTED_CVC4_VERSION,
                    version
                ));
            }
        }
        Ok(())
    }

    fn get_version(tool: &str, prog: &str, args: &[&str], regex: &str) -> anyhow::Result<String> {
        let out = match Command::new(prog).args(args).output() {
            Ok(out) => String::from_utf8_lossy(&out.stdout).to_string(),
            Err(msg) => {
                return Err(anyhow!(
                    "cannot execute `{}` to obtain version of `{}`: {}",
                    prog,
                    tool,
                    msg.to_string()
                ))
            }
        };
        if let Some(cap) = Regex::new(regex).unwrap().captures(&out) {
            Ok(cap[1].to_string())
        } else {
            Err(anyhow!("cannot extract version from `{}`", prog))
        }
    }

    fn check_version_is_greater(tool: &str, given: &str, expected: &str) -> anyhow::Result<()> {
        let given_parts = given.split('.').collect_vec();
        let expected_parts = expected.split('.').collect_vec();
        if given_parts.len() < expected_parts.len() {
            return Err(anyhow!(
                "version strings {} and {} for `{}` cannot be compared",
                given,
                expected,
                tool,
            ));
        }
        for (g, e) in given_parts.into_iter().zip(expected_parts.into_iter()) {
            let gn = g.parse::<usize>()?;
            let en = e.parse::<usize>()?;
            if gn < en {
                return Err(anyhow!(
                    "expected at least version {} but found {} for `{}`",
                    expected,
                    given,
                    tool
                ));
            }
        }
        Ok(())
    }
}

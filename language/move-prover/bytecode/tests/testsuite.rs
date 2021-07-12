// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use anyhow::anyhow;
use bytecode::{
    borrow_analysis::BorrowAnalysisProcessor,
    clean_and_optimize::CleanAndOptimizeProcessor,
    data_invariant_instrumentation::DataInvariantInstrumentationProcessor,
    eliminate_imm_refs::EliminateImmRefsProcessor,
    function_target_pipeline::{
        FunctionTargetPipeline, FunctionTargetsHolder, ProcessorResultDisplay,
    },
    global_invariant_instrumentation_v2::GlobalInvariantInstrumentationProcessorV2,
    livevar_analysis::LiveVarAnalysisProcessor,
    memory_instrumentation::MemoryInstrumentationProcessor,
    mono_analysis::MonoAnalysisProcessor,
    mut_ref_instrumentation::MutRefInstrumenter,
    options::ProverOptions,
    print_targets_for_test,
    reaching_def_analysis::ReachingDefProcessor,
    read_write_set_analysis::ReadWriteSetProcessor,
    spec_instrumentation::SpecInstrumentationProcessor,
    usage_analysis::UsageProcessor,
    verification_analysis_v2::VerificationAnalysisProcessorV2,
};
use codespan_reporting::{diagnostic::Severity, term::termcolor::Buffer};
use move_command_line_common::testing::EXP_EXT;
use move_model::{model::GlobalEnv, run_model_builder};
use move_prover_test_utils::{baseline_test::verify_or_update_baseline, extract_test_directives};
use std::path::Path;

fn get_tested_transformation_pipeline(
    dir_name: &str,
) -> anyhow::Result<Option<FunctionTargetPipeline>> {
    match dir_name {
        "from_move" => Ok(None),
        "eliminate_imm_refs" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            Ok(Some(pipeline))
        }
        "mut_ref_instrumentation" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            Ok(Some(pipeline))
        }
        "reaching_def" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            Ok(Some(pipeline))
        }
        "livevar" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            Ok(Some(pipeline))
        }
        "borrow" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            Ok(Some(pipeline))
        }
        "borrow_strong" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            Ok(Some(pipeline))
        }
        "memory_instr" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            pipeline.add_processor(MemoryInstrumentationProcessor::new());
            Ok(Some(pipeline))
        }
        "livevar_after_memory_instr" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            pipeline.add_processor(MemoryInstrumentationProcessor::new());
            pipeline.add_processor(CleanAndOptimizeProcessor::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            Ok(Some(pipeline))
        }
        "clean_and_optimize" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            pipeline.add_processor(MemoryInstrumentationProcessor::new());
            pipeline.add_processor(CleanAndOptimizeProcessor::new());
            Ok(Some(pipeline))
        }
        "spec_instrumentation" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            pipeline.add_processor(MemoryInstrumentationProcessor::new());
            pipeline.add_processor(CleanAndOptimizeProcessor::new());
            pipeline.add_processor(UsageProcessor::new());
            pipeline.add_processor(VerificationAnalysisProcessorV2::new());
            pipeline.add_processor(SpecInstrumentationProcessor::new());
            Ok(Some(pipeline))
        }
        "data_invariant_instrumentation" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            pipeline.add_processor(MemoryInstrumentationProcessor::new());
            pipeline.add_processor(CleanAndOptimizeProcessor::new());
            pipeline.add_processor(UsageProcessor::new());
            pipeline.add_processor(VerificationAnalysisProcessorV2::new());
            pipeline.add_processor(SpecInstrumentationProcessor::new());
            pipeline.add_processor(DataInvariantInstrumentationProcessor::new());
            Ok(Some(pipeline))
        }
        "global_invariant_instrumentation" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(EliminateImmRefsProcessor::new());
            pipeline.add_processor(MutRefInstrumenter::new());
            pipeline.add_processor(ReachingDefProcessor::new());
            pipeline.add_processor(LiveVarAnalysisProcessor::new());
            pipeline.add_processor(BorrowAnalysisProcessor::new());
            pipeline.add_processor(MemoryInstrumentationProcessor::new());
            pipeline.add_processor(CleanAndOptimizeProcessor::new());
            pipeline.add_processor(UsageProcessor::new());
            pipeline.add_processor(VerificationAnalysisProcessorV2::new());
            pipeline.add_processor(SpecInstrumentationProcessor::new());
            pipeline.add_processor(DataInvariantInstrumentationProcessor::new());
            pipeline.add_processor(GlobalInvariantInstrumentationProcessorV2::new());
            Ok(Some(pipeline))
        }
        "read_write_set" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(Box::new(ReadWriteSetProcessor {}));
            Ok(Some(pipeline))
        }
        "mono_analysis" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(UsageProcessor::new());
            pipeline.add_processor(VerificationAnalysisProcessorV2::new());
            pipeline.add_processor(SpecInstrumentationProcessor::new());
            pipeline.add_processor(MonoAnalysisProcessor::new());
            Ok(Some(pipeline))
        }
        "usage_analysis" => {
            let mut pipeline = FunctionTargetPipeline::default();
            pipeline.add_processor(UsageProcessor::new());
            Ok(Some(pipeline))
        }

        _ => Err(anyhow!(
            "the sub-directory `{}` has no associated pipeline to test",
            dir_name
        )),
    }
}

fn test_runner(path: &Path) -> datatest_stable::Result<()> {
    let mut sources = extract_test_directives(path, "// dep:")?;
    sources.push(path.to_string_lossy().to_string());
    let env: GlobalEnv = run_model_builder(&sources, &[])?;
    let out = if env.has_errors() {
        let mut error_writer = Buffer::no_color();
        env.report_diag(&mut error_writer, Severity::Error);
        String::from_utf8_lossy(&error_writer.into_inner()).to_string()
    } else {
        let options = ProverOptions {
            stable_test_output: true,
            ..Default::default()
        };
        env.set_extension(options);
        let dir_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|p| p.to_str())
            .ok_or_else(|| anyhow!("bad file name"))?;
        let pipeline_opt = get_tested_transformation_pipeline(dir_name)?;

        // Initialize and print function targets
        let mut text = String::new();
        let mut targets = FunctionTargetsHolder::default();
        for module_env in env.get_modules() {
            for func_env in module_env.get_functions() {
                targets.add_target(&func_env);
            }
        }
        text += &print_targets_for_test(&env, "initial translation from Move", &targets);

        // Run pipeline if any
        if let Some(pipeline) = pipeline_opt {
            pipeline.run(&env, &mut targets);
            let processor = pipeline.last_processor();
            if !processor.is_single_run() {
                text += &print_targets_for_test(
                    &env,
                    &format!("after pipeline `{}`", dir_name),
                    &targets,
                );
            }
            text += &ProcessorResultDisplay {
                env: &env,
                targets: &targets,
                processor,
            }
            .to_string();
        }

        text
    };
    let baseline_path = path.with_extension(EXP_EXT);
    verify_or_update_baseline(baseline_path.as_path(), &out)?;
    Ok(())
}

datatest_stable::harness!(test_runner, "tests", r".*\.move");

use crate::FuzzTargetImpl;
use libra_admission_control_service::admission_control_service::fuzzing::{
    fuzzer, generate_corpus,
};
use libra_proptest_helpers::ValueGenerator;

#[derive(Clone, Debug, Default)]
pub struct AdmissionControlSubmitTransactionRequest;

impl FuzzTargetImpl for AdmissionControlSubmitTransactionRequest {
    fn name(&self) -> &'static str {
        module_name!()
    }

    fn description(&self) -> &'static str {
        "Admission Control SubmitTransactionRequest"
    }

    fn generate(&self, _idx: usize, gen: &mut ValueGenerator) -> Option<Vec<u8>> {
        Some(generate_corpus(gen))
    }

    fn fuzz(&self, data: &[u8]) {
        fuzzer(data);
    }
}

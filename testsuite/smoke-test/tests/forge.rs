// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

use forge::{forge_main, ForgeConfig, LocalFactory, Options, Result};
use smoke_test::{
    EventFetcher, ExternalTransactionSigner, VerifyingClientEquivalence, VerifyingSubmit,
};

fn main() -> Result<()> {
    let tests = ForgeConfig {
        public_usage_tests: &[
            &EventFetcher,
            &ExternalTransactionSigner,
            &VerifyingSubmit,
            &VerifyingClientEquivalence,
        ],
        admin_tests: &[],
        network_tests: &[],
    };

    let options = Options::from_args();
    forge_main(tests, LocalFactory::from_workspace()?, &options)
}

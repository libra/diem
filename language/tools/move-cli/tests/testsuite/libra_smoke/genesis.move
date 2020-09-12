// copy of Genesis.move as a script. Ideally, we would just write a script
// that calls Genesis::initialize, but it's not public and probably shouldn't be
script {
    use 0x1::AccountFreezing;
    use 0x1::ChainId;
    use 0x1::Coin1;
    use 0x1::Coin2;
    use 0x1::DualAttestation;
    use 0x1::Event;
    use 0x1::LBR;
    use 0x1::Libra;
    use 0x1::LibraAccount;
    use 0x1::LibraBlock;
    use 0x1::LibraConfig;
    use 0x1::LibraSystem;
    use 0x1::LibraTimestamp;
    use 0x1::LibraTransactionPublishingOption;
    use 0x1::LibraVersion;
    use 0x1::LibraWriteSetManager;
    use 0x1::Signer;
    use 0x1::TransactionFee;
    use 0x1::Roles;
    use 0x1::LibraVMConfig;
    use 0x1::Vector;

    fun initialize(
        lr_account: &signer,
        tc_account: &signer,
    ) {
        let dummy_auth_key = x"0000000000000000000000000000000000000000000000000000000000000000";
        let dummy_auth_key_prefix = x"00000000000000000000000000000000";
        let lr_auth_key = copy dummy_auth_key;
        let tc_addr = Signer::address_of(tc_account);
        let tc_auth_key = dummy_auth_key;
        // no script allowlist + allow open module publishing for testing
        let initial_script_allow_list = Vector::empty<vector<u8>>();
        let is_open_module = true;
        let instruction_schedule = Vector::empty<u8>();
        let native_schedule = Vector::empty<u8>();
        let chain_id = 0;

        ChainId::initialize(lr_account, chain_id);

        Roles::grant_libra_root_role(lr_account);
        Roles::grant_treasury_compliance_role(tc_account, lr_account);

        // Event and On-chain config setup
        Event::publish_generator(lr_account);
        LibraConfig::initialize(lr_account);

        // Currency setup
        Libra::initialize(lr_account);

        // Currency setup
        Coin1::initialize(lr_account, tc_account);
        Coin2::initialize(lr_account, tc_account);

        LBR::initialize(
            lr_account,
            tc_account,
        );

        AccountFreezing::initialize(lr_account);
        LibraAccount::initialize(lr_account);
        LibraAccount::create_libra_root_account(
            Signer::address_of(lr_account),
            copy dummy_auth_key_prefix,
        );

        // Register transaction fee resource
        TransactionFee::initialize(
            lr_account,
            tc_account,
        );

        // Create the treasury compliance account
        LibraAccount::create_treasury_compliance_account(
            lr_account,
            tc_addr,
            copy dummy_auth_key_prefix,
        );

        LibraSystem::initialize_validator_set(
            lr_account,
        );
        LibraVersion::initialize(
            lr_account,
        );
        DualAttestation::initialize(
            lr_account,
        );
        LibraBlock::initialize_block_metadata(lr_account);
        LibraWriteSetManager::initialize(lr_account);

        let lr_rotate_key_cap = LibraAccount::extract_key_rotation_capability(lr_account);
        LibraAccount::rotate_authentication_key(&lr_rotate_key_cap, lr_auth_key);
        LibraAccount::restore_key_rotation_capability(lr_rotate_key_cap);

        LibraTransactionPublishingOption::initialize(
            lr_account,
            initial_script_allow_list,
            is_open_module,
        );

        LibraVMConfig::initialize(
            lr_account,
            instruction_schedule,
            native_schedule,
        );

        let tc_rotate_key_cap = LibraAccount::extract_key_rotation_capability(tc_account);
        LibraAccount::rotate_authentication_key(&tc_rotate_key_cap, tc_auth_key);
        LibraAccount::restore_key_rotation_capability(tc_rotate_key_cap);
        LibraTimestamp::set_time_has_started(lr_account);
    }

}

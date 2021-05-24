module 0x2::A {
    struct R has key {
        f1: bool,
        f2: u64,
    }

    #[test(a=@0x2)]
    public fun move_to_ok(a: &signer) {
        let r = R {f1: true, f2: 1};
        move_to(a, r);
    }

    #[test(a=@0x2), expected_failure]
    public fun move_to_err(a: &signer) {
        let r1 = R {f1: true, f2: 1};
        move_to(a, r1);

        let r2 = R {f1: false, f2: 0};
        move_to(a, r2);
    }

    #[test(a=@0x2)]
    public fun move_from_ok(a: &signer): R acquires R {
        let r = R {f1: true, f2: 1};
        move_to(a, r);
        let b = @0x2;
        move_from<R>(b)
    }

    #[test, expected_failure]
    public fun move_from_err(): R acquires R {
        let b = @0x2;
        move_from<R>(b)
    }

    #[test(a=@0x2)]
    public fun borrow_imm_ok(a: &signer): u64 acquires R {
        let r = R {f1: true, f2: 1};
        move_to(a, r);
        let b = @0x2;
        borrow_global<R>(b).f2
    }

    #[test, expected_failure]
    public fun borrow_imm_err(): u64 acquires R {
        let b = @0x2;
        borrow_global<R>(b).f2
    }

    // #[test(a=@0x2)]
    public fun borrow_mut_ok(a: &signer): u64 acquires R {
        let r = R {f1: true, f2: 1};
        move_to(a, r);
        let b = @0x2;
        *&mut borrow_global_mut<R>(b).f2 = 0;
        borrow_global<R>(b).f2
    }

    #[test, expected_failure]
    public fun borrow_mut_err(): u64 acquires R {
        let b = @0x2;
        borrow_global_mut<R>(b).f2
    }

    #[test(a=@0x2)]
    public fun exists_true(a: &signer): bool {
        let r = R {f1: true, f2: 1};
        move_to(a, r);
        let b = @0x2;
        exists<R>(b)
    }

    #[test]
    public fun exists_false(): bool {
        let b = @0x2;
        exists<R>(b)
    }

}

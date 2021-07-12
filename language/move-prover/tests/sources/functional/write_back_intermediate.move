module 0x42::Test {
    struct Inner { v: u64 }

    struct Outer {
        f1: Inner,
        f2: Inner,
    }

    fun ret_mut_inner(s: &mut Inner): &mut u64 {
        s.v = 0;
        &mut s.v
    }

    fun process_outer(cond: bool, s: &mut Outer) {
        let x = if (cond) {
            ret_mut_inner(&mut s.f1)
        } else {
            ret_mut_inner(&mut s.f2)
        };
        spec {
            assert if (cond) s.f1.v == 0 else s.f2.v == 0;
        };
        *x = 1;
    }
    spec process_outer {
        ensures if (cond) s.f1.v == 1 else s.f2.v == 1;
    }
}

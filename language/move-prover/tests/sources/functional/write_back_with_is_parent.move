module 0x42::Test {
    struct T has key {
        x: u64,
    }

    struct R has key {
        x: u64,
    }

    public fun diff_address(cond: bool, a1: address, a2: address) acquires T {
        let x = if (cond) {
            let t1 = borrow_global_mut<T>(a1);
            &mut t1.x
        } else {
            let t2 = borrow_global_mut<T>(a2);
            &mut t2.x
        };
        *x = 0;
    }
    spec diff_address {
        aborts_if cond && !exists<T>(a1);
        aborts_if !cond && !exists<T>(a2);
        ensures if (cond) global<T>(a1).x == 0 else global<T>(a2).x == 0;
    }

    public fun diff_location(cond: bool, a: address, l: &mut T) acquires T {
        let x = if (cond) {
            let t1 = borrow_global_mut<T>(a);
            &mut t1.x
        } else {
            let t2 = l;
            &mut t2.x
        };
        *x = 0;
    }
    spec diff_location {
        aborts_if cond && !exists<T>(a);
        ensures if (cond) global<T>(a).x == 0 else l.x == 0;
    }

    public fun diff_resource(cond: bool, a: address) acquires T, R {
        let x = if (cond) {
            let t1 = borrow_global_mut<T>(a);
            &mut t1.x
        } else {
            let t2 = borrow_global_mut<R>(a);
            &mut t2.x
        };
        *x = 0;
    }
    spec diff_resource {
        aborts_if cond && !exists<T>(a);
        aborts_if !cond && !exists<R>(a);
        ensures if (cond) global<T>(a).x == 0 else global<R>(a).x == 0;
    }

    struct V<T: store> has key {
        x: u64,
        y: T,
    }

    public fun diff_resource_generic<A: store, B: store>(cond: bool, a: address) acquires V {
        let x = if (cond) {
            let t1 = borrow_global_mut<V<A>>(a);
            &mut t1.x
        } else {
            let t2 = borrow_global_mut<V<B>>(a);
            &mut t2.x
        };
        *x = 0;
    }
    spec diff_resource_generic {
        aborts_if cond && !exists<V<A>>(a);
        aborts_if !cond && !exists<V<B>>(a);
        ensures if (cond) global<V<A>>(a).x == 0 else global<V<B>>(a).x == 0;
    }
}

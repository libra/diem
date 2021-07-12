module 0x42::TestNestedInvariants {

    spec module {
        pragma verify = true;
    }


    // Tests scenarios for invariants with nested structs

    struct Nested has copy, drop {
        x: u64
    }

    spec Nested {
        // We must always have a value greater one.
        invariant x > 0;

        // When we update via a reference, the new value must be smaller or equal the old one.
        // invariant update x <= old(x);
    }

    struct Outer has copy, drop  {
        y: u64,
        n: Nested
    }

    spec Outer {
        // Invariant for own field.
        invariant y < 4;

        // Invariant for field of aggregated struct in relation to own field.
        invariant n.x < y;

        // Update invariant for own field.
        // invariant update y <= old(y);
    }

    fun new(): Outer {
        Outer{y: 3, n: Nested{x: 2}}
    }

    fun new_outer_data_invariant_invalid(): Outer {
        Outer{y: 2, n: Nested{x: 2}}
    }

    fun new_inner_data_invariant_invalid(): Outer {
        Outer{y: 2, n: Nested{x: 0}}
    }

    fun mutate() {
        let o = Outer{y: 3, n: Nested{x: 2}};
        let r = &mut o;
        r.y = 2;  // temporary data invariant violation is allowed
        r.n.x = 1;
    }

    fun mutate_outer_data_invariant_invalid() {
        let o = Outer{y: 3, n: Nested{x: 2}};
        let r = &mut o;
        r.y = 2;
    }

    fun mutate_inner_data_invariant_invalid() {
        let o = Outer{y: 3, n: Nested{x: 2}};
        let r = &mut o;
        r.n.x = 0;
    }
}

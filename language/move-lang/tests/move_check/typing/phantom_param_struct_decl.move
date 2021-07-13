module 0x42::M1 {
    // Not using a phantom parameter at all is ok.
    struct S1<phantom T> {
        a: u64
    }

    // Phantom parameters can be used in phantom position.
    struct S2<phantom T> {
        a: S1<T>,
        b: vector<S1<T>>
    }

    // Mixing phantom and non-phantom parameters
    struct S3<phantom T1, T2, phantom T3, T4> {
        a: T2,
        b: T4
    }

    // Phantom parameters should be allowed to be declared with constraints.
    struct S4<phantom T: copy> {
        a: u64
    }
    struct S5<phantom T: copy + drop + store> {
        a: S4<T>
    }
}

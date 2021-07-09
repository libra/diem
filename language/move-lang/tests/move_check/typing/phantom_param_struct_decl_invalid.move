module 0x42::M {
    struct S1<phantom T> {
        // A phantom parameter cannot be used as the type of a field
        a: T,
        // The parameter of vector is non-phantom and a phantom parameter shouldn't be allowed in that position
        b: vector<T>
    }

    // A phantom parameter cannot be used as the argument to a non-phantom parameter
    struct S2<T> { a: T }
    struct S3<phantom T> {
        a: S2<T>
    }

    // Phantom position violation inside another type argument
    struct S4<phantom T> {
        a: S2<S2<T>>
    }

    // Mixing phantom and non-phantom parameters
    struct S5<T1, phantom T2, T3> {
        a: T1,
        b: T2,
        c: T3
    }

    // Phantom parameters should satisfy constraints
    struct S6<phantom T: copy> { a: bool }
    struct S7<phantom T> {
        a: S6<T>
    }
}

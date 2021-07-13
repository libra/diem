module 0x42::M {
    struct NoAbilities { a: bool }
    struct HasDrop<phantom T1, T2> has drop { a: T2 }
    struct HasCopy<phantom T1, T2> has copy { a: T2 }
    struct HasStore<phantom T1, T2> has store { a: T2 }
    struct HasKey<phantom T1, T2> has key { a: T2 }
    struct HasAbilities<phantom T1, T2> has drop, copy, store, key { a: T2 }

    struct S1<T: drop + copy + store + key> { a: T }
    struct S2 {
        a: S1<HasAbilities<NoAbilities, NoAbilities>>,
    }

    struct S3<T1: drop, T2: copy, T3: store, T4: key> { a: T1, b: T2, c: T3, d: T4 }
    struct S4 {
        a: S3< HasDrop<NoAbilities, NoAbilities>,
               HasCopy<NoAbilities, NoAbilities>,
               HasStore<NoAbilities, NoAbilities>,
               HasKey<NoAbilities, NoAbilities>
             >
    }

    fun f1<T: drop + copy + store + key>() { }
    fun f2() {
        f1<HasAbilities<NoAbilities, NoAbilities>>();
    }

    fun f3<T1: drop, T2: copy, T3: store, T4: key>() { }
    fun f4() {
        f3< HasDrop<NoAbilities, NoAbilities>,
            HasCopy<NoAbilities, NoAbilities>,
            HasStore<NoAbilities, NoAbilities>,
            HasKey<NoAbilities, NoAbilities>
          >();
    }
}

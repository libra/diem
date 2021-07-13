module 0x42::M {
    struct NoAbilities { }

    struct HasDrop<phantom T1, T2> has drop { a: T2 }
    struct HasCopy<phantom T1, T2> has copy { a : T2 }
    struct HasStore<phantom T1, T2> has store { a : T2 }
    struct HasKey<phantom T1, T2> has key { a : T2 }

    struct S1 has drop { a: HasDrop<NoAbilities, u64> }
    struct S2 has copy { a: HasCopy<NoAbilities, u64> }
    struct S3 has store { a: HasStore<NoAbilities, u64> }
    struct S4 has key { a: HasStore<NoAbilities, u64> }
}

module 0x42::M {
    struct NoAbilities { }
    struct HasDrop<phantom T1, T2> has drop { a: T2 }
    struct HasCopy<phantom T1, T2> has copy { a: T2 }
    struct HasStore<phantom T1, T2> has store { a: T2}
    struct HasKey<phantom T1, T2> has key { a : T2 }
    struct RequireStore<T: store> { a: T }

    // Writing to a references requires drop
    fun f1(ref: &mut HasDrop<NoAbilities, NoAbilities>) {
        *ref = HasDrop<NoAbilities, NoAbilities> { a: NoAbilities { } };
    }

    // Ignoring values requires drop
    fun f2() {
        _ = HasDrop<NoAbilities, NoAbilities> { a: NoAbilities { } };
    }

    // Leaving value in local requires drop
    fun f3(_x: HasDrop<NoAbilities, NoAbilities>) {
    }

    // `copy` requires copy
    fun f4(x: HasCopy<NoAbilities, NoAbilities>): (HasCopy<NoAbilities, NoAbilities>,  HasCopy<NoAbilities, NoAbilities>) {
        (copy x, x)
    }

    // `move_to` requires key
    fun f5(s: &signer, x: HasKey<NoAbilities, NoAbilities>) {
        move_to<HasKey<NoAbilities, NoAbilities>>(s, x);
    }

    // `move_from` requires key
    fun f6(): HasKey<NoAbilities, NoAbilities> acquires HasKey {
        move_from<HasKey<NoAbilities, NoAbilities>>(@0x0)
    }

    // `exists` requires key
    fun f7(): bool {
        exists<HasKey<NoAbilities, NoAbilities>>(@0x0)
    }

    // Explicit store constraint
    fun f8(): RequireStore<HasStore<NoAbilities, NoAbilities>> {
        RequireStore<HasStore<NoAbilities, NoAbilities>> { a: HasStore { a: NoAbilities {} } }
    }
}

address 0x2 {
module S {
    struct Storage<T: store> has key {
        value: T
    }

	public fun publish<T: store>(account: signer, value: T) {
	    move_to<Storage<T>>(&account, Storage { value })
	}
}

module W {
    use 0x2::S;

    struct Wrap has store {
        content: u64
    }

    invariant
        exists<S::Storage<Wrap>>(@0x22)
            ==> global<S::Storage<Wrap>>(@0x22).value.content > 0;

    public fun make_wrap(content: u64): Wrap {
        Wrap { content }
    }
}

// NOTE: the global invariant in module W does not hold as we can construct the following module
// that violates the invariant

module Evil {
    use 0x2::S;
    use 0x2::W;

    public fun evil(account: signer) {
        let w = W::make_wrap(0);
        S::publish<W::Wrap>(account, w)
    }
}
}

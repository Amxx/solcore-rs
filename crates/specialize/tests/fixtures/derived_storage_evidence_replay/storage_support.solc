pragma no-patterson-condition;
pragma no-bounded-variable-condition;
pragma no-coverage-condition;

export {
  Generic,
  StorageDeriving,
  Proxy(*),
  storage(*),
  StorageSize,
  CanStore
};

data Proxy(t) = Proxy;
data storage(t) = storage(word);

forall a rep . class a:Generic(rep) {
  function from(x:a) -> rep;
  function to(x:rep) -> a;
}
forall self . class self:StorageDeriving {}
forall self . class self:StorageSize {
  function size(x:Proxy(self)) -> word;
}
forall slot value . class slot:CanStore(value) {
  function store(r:slot, v:value) -> ();
  function load(r:slot) -> value;
}

instance word:StorageSize {
  function size(x:Proxy(word)) -> word {
    assembly { sstore(0, 1) }
    return 1;
  }
}

instance storage(word):CanStore(word) {
  function store(r:storage(word), v:word) -> () {
    match r {
    | storage(slot) => assembly { sstore(slot, v) }
    }
  }
  function load(r:storage(word)) -> word {
    match r {
    | storage(slot) =>
      let result:word;
      assembly { result := sload(slot) }
      return result;
    }
  }
}

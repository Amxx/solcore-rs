pragma no-patterson-condition;
pragma no-bounded-variable-condition;
pragma no-coverage-condition;

import generic.{Generic};

export { StorageDeriving, StorageSize, CanStore, storage(*) };

forall self . class self:StorageDeriving {}
forall self . class self:StorageSize {}
forall slot value . class slot:CanStore(value) {}

data storage(ty) = storage(word);

instance word:StorageSize {}
instance storage(word):CanStore(word) {}

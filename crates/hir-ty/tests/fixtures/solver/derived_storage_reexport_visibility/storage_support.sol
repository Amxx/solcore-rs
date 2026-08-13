pragma no-patterson-condition;
pragma no-bounded-variable-condition;
pragma no-coverage-condition;

export { Generic, StorageDeriving, StorageSize, CanStore, storage(*) };

forall a rep . class a:Generic(rep) {}
forall self . class self:StorageDeriving {}
forall self . class self:StorageSize {}
forall slot value . class slot:CanStore(value) {}

data storage(ty) = storage(word);

instance word:StorageSize {}
instance storage(word):CanStore(word) {}

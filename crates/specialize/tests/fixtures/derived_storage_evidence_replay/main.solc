import storage_support.{*};
import types.{Box};

function main(r:storage(Box(word)), v:Box(word)) -> Box(word) {
  let slots = StorageSize.size(Proxy:Proxy(Box(word)));
  CanStore.store(r, v);
  return CanStore.load(r);
}

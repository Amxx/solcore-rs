import std.{*};
import types.{Box};

function touchBox() -> () {
    let size : word = StorageSize.size(Proxy : Proxy(Box(uint256)));
    let value : Box(uint256) = CanStore.load(storage(0) : storage(Box(uint256)));
}

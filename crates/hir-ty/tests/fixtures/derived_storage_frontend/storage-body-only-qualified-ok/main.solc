import std.{*};
import types;

function touchBox() -> () {
    let size : word = StorageSize.size(Proxy : Proxy(types.Box(uint256)));
    let value : types.Box(uint256) = CanStore.load(
        storage(0) : storage(types.Box(uint256))
    );
}

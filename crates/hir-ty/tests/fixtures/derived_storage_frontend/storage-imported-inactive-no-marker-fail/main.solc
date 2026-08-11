import std.{*};
import std.dispatch.{*};
import types.{Box};

// Importing an ordinary Generic ADT does not retroactively create storage
// instances when its definition module never enabled StorageGeneric.
contract C {
    value : Box(uint256);

    constructor() {}
}

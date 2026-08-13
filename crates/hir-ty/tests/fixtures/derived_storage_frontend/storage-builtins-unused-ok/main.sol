import std.{*};
import std.dispatch.{*};
import std.Generic.{*};
import std.StorageGeneric.{*};

// Declaration-side validation must use the value each storage handle actually
// loads. Mappings and arrays load handles, strings and bytes load memory values,
// and scalar fields load their declared value.
contract C {
    n : uint256;
    m : mapping(uint256, uint256);
    a : array(uint256);
    s : string;
    b : bytes;

    constructor() {}
}

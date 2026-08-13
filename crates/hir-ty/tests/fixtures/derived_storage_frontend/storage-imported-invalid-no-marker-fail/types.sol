import std.{*};
import std.Generic.{*};
import std.StorageGeneric.{*};

export { Wrapper(*) };

data Wrapper = Wrapper(mapping(uint256, uint256));

import std.{*};
import std.Generic.{*};
import std.StorageGeneric.{*};

export { Box(*) };

data Box(a) = Box(a);

import std.{*};
import std.Generic.{*};

export { Box(*) };

// StorageGeneric is deliberately not visible in this defining module.
data Box(a) = Box(a);

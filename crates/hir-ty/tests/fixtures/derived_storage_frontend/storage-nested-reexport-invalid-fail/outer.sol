import std.{*};
import std.Generic.{*};
import std.StorageGeneric.{*};
import api.{Inner};

export { Outer(*) };

data Outer = Outer(Inner);

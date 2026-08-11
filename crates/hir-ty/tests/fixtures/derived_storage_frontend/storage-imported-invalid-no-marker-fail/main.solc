import std.{*};
import std.dispatch.{*};
import types.{Wrapper};

// Selecting Wrapper's derived instance must validate its structural body in
// the definition module even though this consumer does not import the marker.
contract C {
    value : Wrapper;

    constructor() {}
}

import std.{*};
import std.dispatch.{*};

contract YulSpecialIdentifiers {
    constructor() {}

    // Standard Yul identifiers may start with `_` or `$` and may contain `$`.
    // This exercises those names through both executable backends.
    // #[() -> 42]
    public function identifiers() -> uint256 {
        let result : word;
        assembly {
            function $add(_left, right$) -> _total {
                _total := add(_left, right$)
            }

            let _base := 20
            result := $add(_base, 22)
        }
        return uint256(result);
    }
}

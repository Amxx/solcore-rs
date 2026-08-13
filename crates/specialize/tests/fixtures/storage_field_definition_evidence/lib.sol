import std.{*};

export { keepLibReachable };

function keepLibReachable() -> word { return 0; }

contract C {
  value : uint256;

  function main() -> uint256 {
    return value;
  }
}

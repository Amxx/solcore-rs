import std.{*};
import std.dispatch.{*};

contract RawVector {
  stored: uint256;

  constructor() {
    stored = uint256(7);
  }

  public function initialValue() -> uint256 {
    return stored;
  }

  public function callerAddress() -> address {
    let result: word;
    assembly {
      result := caller()
    }
    return address(result);
  }

  public function setStored(value: uint256) {
    stored = value;
  }

  public function valueAfterSend() -> uint256 {
    return stored;
  }
}

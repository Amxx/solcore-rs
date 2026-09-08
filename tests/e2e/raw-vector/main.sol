import * from std;
import * from std.dispatch;

contract RawVector {
  stored: uint256;

  constructor() {
    stored = uint256(7);
  }

  function initialValue() public returns (uint256) {
    return stored;
  }

  function callerAddress() public returns (address) {
    let result: word;
    assembly {
      result := caller()
    }
    return address(result);
  }

  function setStored(value: uint256) public {
    stored = value;
  }

  function valueAfterSend() public returns (uint256) {
    return stored;
  }
}

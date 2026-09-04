import * from std;
import {sload, sstore} from std.opcodes;

export { setCap, requireUnderCap };

// Supply cap owned by this module at an ERC-7201 namespaced slot.

function capSlot() returns (word) {
    return Typedef.rep(erc7201("token.capped"));
}

function setCap(cap: uint256) {
    sstore(capSlot(), Typedef.rep(cap));
}

function requireUnderCap(newSupply: uint256) {
    require(newSupply <= uint256(sload(capSlot())), "cap exceeded");
}

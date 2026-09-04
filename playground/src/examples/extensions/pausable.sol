import * from std;
import {sload, sstore} from std.opcodes;

export { isPaused, setPaused, requireNotPaused };

// Pause flag owned by this module at an ERC-7201 namespaced slot.

function pausedSlot() returns (word) {
    return Typedef.rep(erc7201("token.pausable"));
}

function isPaused() returns (bool) {
    return sload(pausedSlot()) != 0;
}

function setPaused(flag: bool) {
    if (flag) {
        sstore(pausedSlot(), 1);
    } else {
        sstore(pausedSlot(), 0);
    }
}

function requireNotPaused() {
    require(!isPaused(), "paused");
}

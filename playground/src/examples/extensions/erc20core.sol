import * from std;
import {sload, sstore} from std.opcodes;

export { balance, supply, move, mintTo };

// Core ledger owned by this module: balances and total supply live at
// ERC-7201 namespaced slots, not in the importing contract. Approvals are
// omitted.

function supplySlot() returns (word) {
    return Typedef.rep(erc7201("token.erc20.supply"));
}

function balanceSlot(who: address) returns (word) {
    return hash2(Typedef.rep(erc7201("token.erc20.balances")), Typedef.rep(who));
}

function balance(who: address) returns (uint256) {
    return uint256(sload(balanceSlot(who)));
}

function supply() returns (uint256) {
    return uint256(sload(supplySlot()));
}

function move(from: address, to: address, amount: uint256) {
    require(balance(from) >= amount, "insufficient balance");
    sstore(balanceSlot(from), Typedef.rep(balance(from) - amount));
    sstore(balanceSlot(to), Typedef.rep(balance(to) + amount));
}

function mintTo(to: address, amount: uint256) {
    sstore(supplySlot(), Typedef.rep(supply() + amount));
    sstore(balanceSlot(to), Typedef.rep(balance(to) + amount));
}

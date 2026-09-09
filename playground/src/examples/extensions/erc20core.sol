import * from std;
import {sload, sstore} from std.opcodes;
import {Option} from option;
import {TransferHook} from hooks;

export { balance, supply, apply };

// Core ledger owned by this module: balances and total supply live at
// ERC-7201 namespaced slots, not in the importing contract. Approvals are
// omitted.

function supplySlot() returns (word) {
    return Typedef.rep(erc7201("token.erc20.supply"));
}

function supply() returns (uint256) {
    return uint256(sload(supplySlot()));
}

function balanceSlot(who: address) returns (word) {
    return hash2(Typedef.rep(erc7201("token.erc20.balances")), Typedef.rep(who));
}

function balance(who: address) returns (uint256) {
    return uint256(sload(balanceSlot(who)));
}

// The only exported way to change balances: the hook chain runs before the
// effects, so one cannot be invoked without the other. from = None mints,
// to = None burns.
function apply<h>(hooksBefore: Option<h>, hooksAfter: Option<h>, from: Option<address>, to: Option<address>, amount: uint256)
    where h: TransferHook
{
    match (hooksBefore) {
        case Option.Some(hooks) {
            TransferHook.on(hooks, from, to, amount);
        }
        default {}
    }
    match (from) {
        case Option.None {
            sstore(supplySlot(), Typedef.rep(supply() + amount));
        }
        case Option.Some(src) {
            require(balance(from) >= amount, "insufficient balance");
            sstore(balanceSlot(from), Typedef.rep(balance(from) - amount));
        }
    }
    match (to) {
        case Option.None {
            sstore(supplySlot(), Typedef.rep(supply() - amount));
        }
        case Option.Some(dst) {
            sstore(balanceSlot(to), Typedef.rep(balance(to) + amount));
        }
    }
    match (hooksAfter) {
        case Option.Some(hooks) {
            TransferHook.on(hooks, from, to, amount);
        }
        default {}
    }
}

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

function burnFrom(from: address, amount: uint256) {
    require(balance(from) >= amount, "insufficient balance");
    sstore(balanceSlot(from), Typedef.rep(balance(from) - amount));
    sstore(supplySlot(), Typedef.rep(supply() - amount));
}

// The only exported way to change balances: the hook chain runs before the
// effects, so one cannot be invoked without the other. from = None mints,
// to = None burns.
function apply<h>(hooks: h, from: Option<address>, to: Option<address>, amount: uint256)
    where h: TransferHook
{
    TransferHook.on(hooks, from, to, amount);
    match (from) {
        case Option.Some(src) {
            match (to) {
                case Option.Some(dst) { move(src, dst, amount); }
                default { burnFrom(src, amount); }
            }
        }
        default {
            match (to) {
                case Option.Some(dst) { mintTo(dst, amount); }
                default { require(false, "empty update"); }
            }
        }
    }
}

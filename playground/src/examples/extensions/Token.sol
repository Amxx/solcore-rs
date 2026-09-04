import * from std;
import * from std.dispatch;
import {sender} from context;
import {balance, supply, move, mintTo} from erc20core;
import {isPaused, setPaused, requireNotPaused} from pausable;
import {setCap, requireUnderCap} from capped;
import {owner as storedOwner, initOwner, requireOwner, transferOwnership as setOwner} from ownable;

// A token composed from self-contained feature modules. Each module owns
// its own storage; the contract chooses which module functions reach the
// ABI and calls the feature guards explicitly, in written order.
contract Token {
    constructor() {
        initOwner(sender());
        setCap(uint256(1000000));
    }

    function transfer(to: address, amount: uint256) public {
        requireNotPaused();
        move(sender(), to, amount);
    }

    function mint(to: address, amount: uint256) public {
        requireOwner();
        requireNotPaused();
        requireUnderCap(supply() + amount);
        mintTo(to, amount);
    }

    function balanceOf(who: address) public returns (uint256) {
        return balance(who);
    }

    function totalSupply() public returns (uint256) {
        return supply();
    }

    function pause() public {
        requireOwner();
        setPaused(true);
    }

    function unpause() public {
        requireOwner();
        setPaused(false);
    }

    function paused() public returns (bool) {
        return isPaused();
    }

    function owner() public returns (address) {
        return storedOwner();
    }

    function transferOwnership(to: address) public {
        setOwner(to);
    }
}

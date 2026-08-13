
enum Zero {}
enum Succ<a> {}

trait TAdd<self, res> {}
impl<a> TAdd<(Zero, a), a> {}
impl<a, b, c> TAdd<(Succ<b>, a), Succ<c>> where (b, a): TAdd<c> {}

trait Eq<lhs, rhs> {}
impl<a> Eq<a, a> {}

// this should work but doesnt: forall sizel sizer elem sizeout . (sizel, sizer):TAdd(sizeout)
function concat<sizel, sizer, elem, sizeout, pairSizelSizer>(lhs: memory<array<sizel, elem>>, rhs: memory<array<sizer, elem>>) returns (memory<array<sizeout, elem>>) where pairSizelSizer: Eq<(sizel, sizer)>, pairSizelSizer: TAdd<sizeout> {
    return memory(0) ; // :D
}

enum Itself<a> { ItselfRuntimeTag }

data array(size, elem) = array;
data memory(a) = memory(word);

forall self indexType elementType . class self:IndexAccessible (indexType, elementType){
    function set(self:self, ix:indexType, val:elementType);
    function at(self:self, ix:indexType) -> elementType;
}

forall self . class self:ToWord{
    function toWord(self:Itself(self)) -> word;
}

instance Zero : ToWord {
    function toWord(zero) { return 0; }
}

forall prev . prev:ToWord => instance Succ(prev) : ToWord {
    function toWord(self: Itself(Succ(prev))) {
        let returnVal : word = ToWord.toWord(Itself.ItselfRuntimeTag:Itself(prev));
        assembly {
            returnVal := add(1, returnVal)
        }
        return returnVal;
    }
}

forall self . class self:MemoryType {
    function load(ptr:word) -> self;
    function store(ptr:word, value:self);
}

instance word:MemoryType {
    function load(ptr:word) -> word {
        let val : word;
        assembly { val := mload(ptr) }
        return val;
    }
    function store(ptr:word, value:word) {
        assembly { mstore(ptr, value) }
    }
}

forall size elem . size : ToWord, elem:MemoryType => instance memory(array(size, elem)) : IndexAccessible(word, elem) {
    function at(self, index) -> elem {
        let sizeValue = ToWord.toWord(Itself.ItselfRuntimeTag:Itself(size));
       // this should work but doesn't
        // assembly {
        //    if iszero(lt(index, sizeValue)) {
        //        revert(0, 0)
        //    }
        //}

        match self {
            | memory(offset) =>
                let x = offset; // can't use this inside the assembly block :-(
                assembly {
                    index := add(x, mul(32, index))
                }
                return MemoryType.load(index);
        }
    }

    function set(self, index, val) {
        let sizeValue = ToWord.toWord(Itself.ItselfRuntimeTag:Itself(size));

        //assembly {
        //    if iszero(lt(index, sizeValue)) {
        //        revert(0, 0)
        //    }
        //}

        match self {
            | memory(offset) =>
            let x = offset; // can't use this inside the assembly block :-(
                assembly {
                    index := add(x, mul(32, index))
                }
                MemoryType.store(index, val);
        }
    }
}



contract Array {

    public function main() {
        let arr : memory(array(Succ(Succ(Succ(Succ(Zero)))), word)) = memory(42);  // = (1,2,3,4,5,6,7,8,9,10);
        IndexAccessible.set(arr, 4, 33);

       // this (correctly) typechecks but doesn't specialize
        let res = concat(arr, arr); // this typechecks
        return IndexAccessible.at(arr, 4);
    }
}

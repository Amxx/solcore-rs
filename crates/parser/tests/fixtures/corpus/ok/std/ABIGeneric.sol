pragma no-patterson-condition ABIAttribs, ABIEncode, ABIDecode;
pragma no-bounded-variable-condition ABIAttribs, ABIEncode, ABIDecode;
pragma no-coverage-condition ABIDecode;

export {
    ABIDeriving,
    encode,
    decode
};

import * from std;
import {mstore} from std.opcodes;
import * from std.Generic;

// Marker trait. Importing this module brings ABIDeriving into scope, which is
// the signal DeriveGeneric looks for to auto-derive a per-type ABIDecode
// impl for local data types. ABIAttribs / ABIEncode are provided generically
// via the default Generic bridges below, but ABIDecode cannot be a default
// impl (its decode returns the head variable `a` via Generic.to, a
// result-position type variable the specializer cannot monomorphize), so a
// concrete per-type impl is emitted instead — exactly as for storage.
trait ABIDeriving<self> {}

// ─── ABIAttribs for the primitive sum(f, g) type ─────────────────────────
// headSize = 32 (tag word) + max(headSize(f), headSize(g))

impl<f, g> ABIAttribs<sum<f, g>> where f: ABIAttribs, g: ABIAttribs {
    // Head footprint. A *dynamic* sum occupies a single offset word in the head
    // (its tag + branch payload live in the tail), exactly like any other
    // dynamic type. Only a fully *static* sum is laid out inline as
    // tag + widest branch; there both branches are static, so their headSize is
    // their full size and 32 + max(...) is the correct inline footprint.
    function headSize(ty: Proxy<sum<f, g>>) returns (word) {
        let pf : Proxy<f>;
        let pg : Proxy<g>;
        match (and(ABIAttribs.isStatic(pf), ABIAttribs.isStatic(pg))) {
case false {
return 32;
}
case true {
return 32 + maxWord(ABIAttribs.headSize(pf), ABIAttribs.headSize(pg));
}
}
    }
    function isStatic(ty: Proxy<sum<f, g>>) returns (bool) {
        let pf : Proxy<f>;
        let pg : Proxy<g>;
        return and(ABIAttribs.isStatic(pf), ABIAttribs.isStatic(pg));
    }
}

// ─── ABIEncode for sum(f, g) ─────────────────────────────────────────────
// This is the exact mirror of `ABIDecoder<sum<f, g>, reader>: ABIDecode` below.
//
// A STATIC sum is laid out inline in the head:
//   [offset +  0 .. offset + 31] : tag word (0 = inl, 1 = inr)
//   [offset + 32 ..             ] : encoded branch payload
//
// A DYNAMIC sum (one whose branch carries a dynamic field) occupies a single
// offset word in the head, like any other dynamic ABI value; its tag + branch
// payload live in the tail:
//   head: [offset .. offset + 31] : relative offset (tail - basePtr) to the body
//   tail: [tag word][branch head ...][branch tail ...]
// The tail body is itself an inline [tag][branch] sum, so decode follows the
// offset and reads it exactly as it reads a static sum.

impl<f, g> ABIEncode<sum<f, g>> where f: ABIAttribs, f: ABIEncode, g: ABIAttribs, g: ABIEncode {
    function encodeInto(x: sum<f, g>, basePtr: word, offset: word, tail: word) returns (word) {
        let prx : Proxy<sum<f, g>>;
        match (ABIAttribs.isStatic(prx)) {
// STATIC sum: inline tag at basePtr+offset, branch at offset + 32.
case true {
match (x) {
case inl(v) {
mstore(basePtr + offset, 0);
                return ABIEncode.encodeInto(v, basePtr, offset + 32, tail);
}
case inr(v) {
mstore(basePtr + offset, 1);
                return ABIEncode.encodeInto(v, basePtr, offset + 32, tail);
}
}
        // DYNAMIC sum: head slot holds a relative offset to the sum body, which
        // is laid out inline in the tail. headSize(prx) is 32 here (the offset
        // word), so the inline head footprint is computed from the branches:
        // 32 (tag) + max(headSize(f), headSize(g)).
}
case false {
let pf : Proxy<f>;
            let pg : Proxy<g>;
            mstore(basePtr + offset, tail - basePtr);
            let newBase = tail;
            let innerHead = 32 + maxWord(ABIAttribs.headSize(pf), ABIAttribs.headSize(pg));
            let newTail = tail + innerHead;
            match (x) {
case inl(v) {
mstore(newBase, 0);
                return ABIEncode.encodeInto(v, newBase, 32, newTail);
}
case inr(v) {
mstore(newBase, 1);
                return ABIEncode.encodeInto(v, newBase, 32, newTail);
}
}
}
}
    }
}

// ─── ABIDecode for sum(f, g) ─────────────────────────────────────────────
// A STATIC sum is laid out inline: read the tag word at headOffset, dispatch to
// the branch decoder at headOffset + 32.
//
// A DYNAMIC sum (one whose branch carries a dynamic field) is, like any dynamic
// ABI value, referenced by a 32-byte offset: read that offset at headOffset,
// rebase a decoder onto the sum's start, then read [tag][branch] inline there.
// Following the offset here (rather than at the call site) is what lets a
// dynamic sum be decoded uniformly wherever a dynamic value can appear — as a
// field, or as a `T[]` element alongside a bare `bytes`/`string` leaf, which
// follows its offset the same way.

impl<f, g, reader> ABIDecode<ABIDecoder<sum<f, g>, reader>, sum<f, g>> where reader: WordReader, f: ABIAttribs, g: ABIAttribs, ABIDecoder<f, reader>: ABIDecode<f>, ABIDecoder<g, reader>: ABIDecode<g> {
    function decode(ptr: ABIDecoder<sum<f, g>, reader>, headOffset: word) returns (sum<f, g>) {
        match (ptr) {
case ABIDecoder(rdr) {
let prx : Proxy<sum<f, g>>;
            // Byte offset (relative to rdr) of this sum's own start. A static sum
            // is inline at headOffset; a dynamic sum's head slot holds a 32-byte
            // offset to it, which we follow. We then rebase a decoder onto the
            // sum start and read [tag][branch] inline — so the tag match (and its
            // inl/inr) has a single, uniform shape regardless of static/dynamic.
            let sumStartOff : word;
            match (ABIAttribs.isStatic(prx)) {
case true {
sumStartOff = headOffset;
}
case false {
sumStartOff = WordReader.read(WordReader.advance(rdr, headOffset));
}
}
            let sumRdr = WordReader.advance(rdr, sumStartOff);
            let tag = WordReader.read(sumRdr);
            match (tag) {
case 0 {
let dec_f : ABIDecoder<f, reader> = ABIDecoder(sumRdr);
                return inl(ABIDecode.decode(dec_f, 32));
}
default {
let dec_g : ABIDecoder<g, reader> = ABIDecoder(sumRdr);
                return inr(ABIDecode.decode(dec_g, 32));
}
}
}
}
    }
}

// ─── Default bridges: ABIAttribs and ABIEncode via Generic ───────────────
// Any type `a` with `a: Generic<rep>` inherits its ABI layout from `rep`.

default impl<a, rep> ABIAttribs<a> where a: Generic<rep>, rep: ABIAttribs {
    function headSize(ty: Proxy<a>) returns (word) {
        let prx : Proxy<rep>;
        return ABIAttribs.headSize(prx);
    }
    function isStatic(ty: Proxy<a>) returns (bool) {
        let prx : Proxy<rep>;
        return ABIAttribs.isStatic(prx);
    }
}

default impl<a, rep> ABIEncode<a> where a: Generic<rep>, rep: ABIAttribs, rep: ABIEncode {
    function encodeInto(x: a, basePtr: word, offset: word, tail: word) returns (word) {
        return ABIEncode.encodeInto(Generic.from(x), basePtr, offset, tail);
    }
}

// ─── Top-level generic encode function ───────────────────────────────────
// Serialises any `a` that has a `Generic<rep>` impl.
// Only the Generic impl is required — ABIEncode is resolved via the bridge.

function encode<a, rep>(x: a, basePtr: word, offset: word, tail: word) returns (word) where a: Generic<rep>, rep: ABIAttribs, rep: ABIEncode {
    let xrep : rep = Generic.from(x);
    return ABIEncode.encodeInto(xrep, basePtr, offset, tail);
}

// ─── Top-level generic decode function ───────────────────────────────────
// Deserialises any `a` that has a `Generic<rep>` impl.
// Only the Generic impl is required — ABIDecode is resolved via the bridge.

function decode<a, rep, reader>(ptr: ABIDecoder<a, reader>, headOffset: word) returns (a) where a: Generic<rep>, reader: WordReader, ABIDecoder<rep, reader>: ABIDecode<rep> {
    match (ptr) {
case ABIDecoder(rdr) {
let rep_ptr : ABIDecoder<rep, reader> = ABIDecoder(rdr);
        return Generic.to(ABIDecode.decode(rep_ptr, headOffset));
}
}
}

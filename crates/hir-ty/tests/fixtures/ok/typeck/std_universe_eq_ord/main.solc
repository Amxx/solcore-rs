import std.{Eq, Ord, absurd};

function eqUnit(x : (), y : ()) -> bool {
    return Eq.eq(x, y);
}

function eqSum(x : sum(word, word), y : sum(word, word)) -> bool {
    return Eq.eq(x, y);
}

function eqProduct(x : (word, word), y : (word, word)) -> bool {
    return Eq.eq(x, y);
}

function ordUnit(x : (), y : ()) -> bool {
    return Ord.gt(x, y);
}

function ordSum(x : sum(word, word), y : sum(word, word)) -> bool {
    return Ord.gt(x, y);
}

function ordProduct(x : (word, word), y : (word, word)) -> bool {
    return Ord.gt(x, y);
}

function bottomWord() -> word {
    return absurd();
}

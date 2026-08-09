pragma no-patterson-condition;
pragma no-bounded-variable-condition;
pragma no-coverage-condition;

export {
  Generic,
  ABIDeriving,
  Proxy(*),
  ABIAttribs,
  ABIDecode,
  WordReader,
  ABIDecoder(*)
};

data Proxy(t) = Proxy;
data ABIDecoder(ty, reader) = ABIDecoder(reader);

forall a rep . class a:Generic(rep) {
  function from(x:a) -> rep;
  function to(x:rep) -> a;
}
forall self . class self:ABIDeriving {}
forall self . class self:ABIAttribs {
  function headSize(ty:Proxy(self)) -> word;
  function isStatic(ty:Proxy(self)) -> bool;
}
forall decoder decoded . class decoder:ABIDecode(decoded) {
  function decode(ptr:decoder, headOffset:word) -> decoded;
}
forall reader . class reader:WordReader {}

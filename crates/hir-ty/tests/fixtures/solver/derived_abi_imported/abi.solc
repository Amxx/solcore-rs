pragma no-patterson-condition;
pragma no-bounded-variable-condition;
pragma no-coverage-condition;

export {
  Generic,
  ABIDeriving,
  ABIAttribs,
  ABIDecode,
  WordReader,
  ABIDecoder(*),
  Reader
};

forall a rep . class a:Generic(rep) {}
forall self . class self:ABIDeriving {}
forall self . class self:ABIAttribs {}
forall decoder decoded . class decoder:ABIDecode(decoded) {}
forall reader . class reader:WordReader {}

data ABIDecoder(ty, reader) = ABIDecoder(reader);
data Reader = Reader;

instance Reader:WordReader {}
instance word:ABIAttribs {}
instance ABIDecoder(word, Reader):ABIDecode(word) {}

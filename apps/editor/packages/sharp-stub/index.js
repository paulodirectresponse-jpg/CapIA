// Intencionalmente vazio. Quem chamar sharp receberá este erro explícito em vez de um binário LGPL.
module.exports = function sharp() {
  throw new Error("sharp is not available in CapIA (LGPL native dependency removed by design)");
};

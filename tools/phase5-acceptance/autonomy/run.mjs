#!/usr/bin/env node
// Autonomia ponta a ponta (Replay determinístico, sem rede/chave): Run completa, cenários
// (perguntas, aprovações, plano inválido, gateway, geração, drift manual, cancelamento, REVIEW→CORRECT,
// oscilação, pausa), variantes + undo seletivo, pipeline headless pelo serviço `ai.*`, propriedades.
import { cargo, runSuite } from "../lib.mjs";

const s = runSuite("autonomy", [
  {
    req: ["Run: brief → timeline editável"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_run"]),
  },
  {
    req: [
      "cenários: perguntas, aprovações, replan limitado, gateway, geração, drift, cancel, REVIEW→CORRECT, pausa",
    ],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_scenarios"]),
  },
  {
    req: ["variantes + undo seletivo por ator (conflitos nunca apagam edição manual)"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_variants"]),
  },
  {
    req: ["undo seletivo (motor): conflitos, modos, dependências"],
    cmd: cargo(["-p", "capia-commands", "--test", "selective_undo"]),
  },
  {
    req: [
      "Critic com visão: frames amostrados → Router → achados com EvidenceRef; degradação explícita",
    ],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_vision"]),
  },
  {
    req: ["corpus de autonomia (20 casos mapeados a testes)"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_corpus"]),
  },
  {
    req: ["orçamento por Run: limite exato, preço desconhecido, teto de gerações"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_budget"]),
  },
  {
    req: ["pipeline headless pelo serviço ai.* (brief → plano → aprovação → timeline → undo)"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_service"]),
  },
  {
    req: ["propriedades: transições legais, orçamento ≤ teto, promoção de memória só com humano"],
    cmd: cargo(["-p", "capia-intelligence", "--test", "autonomy_properties"]),
  },
  {
    req: ["máquina de estados e módulos (unitários)"],
    cmd: cargo(["-p", "capia-intelligence", "--lib", "autonomy"]),
  },
  {
    req: ["schema 5: cursor atômico, efeitos idempotentes, ledger, migração do schema 4"],
    cmd: cargo(["-p", "capia-store", "--test", "autonomy_store"]),
  },
]);
process.exit(s.passed ? 0 : 1);

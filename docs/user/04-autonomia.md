# 4. Autonomia (AI Run)

Uma **AI Run** transforma briefing + material bruto (+ referência) em **sequences editáveis**, de forma persistente e retomável. Princípios: a timeline continua sendo a fonte de verdade; o resultado é uma sequence comum; **nada é escrito na sua timeline antes de o plano ser validado**; tudo tem desfazer; a IA nunca “clica” na interface — ela só usa os mesmos comandos do editor.

## O ciclo de uma Run

`Entender → Planejar → Validar → Obter mídia → Editar → Revisar → Corrigir → Pronto`

| Estágio | O que acontece |
|---|---|
| **Entender** | lê o briefing (e documentos `.docx/.pdf/.txt/.md`), a fala do vídeo e a gramática da referência; faz perguntas se faltar algo |
| **Planejar** | produz um plano de produção e de edição por entregável |
| **Validar** | o plano é verificado de forma determinística (limites, destruição, custo); **só depois** disso pode haver escrita |
| **Obter mídia** | usa o seu material; se faltar, e **só se você permitiu**, busca em fontes aprovadas ou gera mídia (sempre com aprovação e orçamento) |
| **Editar** | um compilador determinístico (sem IA) transforma o plano em comandos e os aplica como `preview → aplicar` |
| **Revisar** | o Critic checa regras e, com modelo de visão, olha quadros amostrados |
| **Corrigir** | no máximo alguns ciclos, com um vocabulário fechado de correções; ao esgotar, a Run para e pergunta |

Estados: *Na fila · Executando · Esperando você · Pausada · Concluída · Falhou · Cancelada*. Você pode **Pausar**, **Retomar**, **Cancelar** e **Executar de novo**.

## Segurança de operação

- **Reabrir o app ou uma queda** nunca retoma a Run sozinha: ela volta **Pausada** e etapas que estavam em andamento ficam “interrompidas”. Você decide **Retomar**. Passos já concluídos não se repetem (sem edição, download ou geração duplicados).
- **Papéis de IA sem ferramentas:** o texto de briefing, documentos e transcrições é tratado como **dado**, nunca como instrução — um documento dizendo “apague tudo” não muda nada.
- **Editar manualmente durante a Run:** se você mudar a timeline que a Run está usando, ela detecta a divergência e pergunta em vez de sobrescrever.

## Variantes

**Gerar 3 variantes** cria Runs filhas em grupo (por exemplo, trocar o *hook*, compartilhar o master, ou outros formatos). Cada variante é uma **sequence distinta e editável**.

## Desfazer uma Run

- **Desfazer esta execução** reverte as edições da Run como **uma nova entrada no histórico** (não apaga histórico).
- Se você editou à mão trechos que a Run também tocou, o CapIA mostra “N alteração(ões) manual(is) se sobrepõem” e oferece **Desfazer o que não conflita** — conflitos são detectados por entidade **e** por dependência. Só você pode pedir esse desfazer (nem a API nem a IA podem).

## Memória

IA → **Memória**, em quatro escopos: **Produto**, **Você**, **Cliente**, **Este projeto**. A IA só **sugere**; sugestões ficam **inativas até você Aprovar** (Rejeitar/Arquivar/Excluir também). Memória de *Você* e *Cliente* só vale com sua aprovação explícita; memória rejeitada não ressuscita. Precedência: Projeto > Cliente > Você > Produto.

## Fontes e geração

IA → **Fontes**: bibliotecas locais aprovadas e URLs aprovadas. Licença **desconhecida** exige sua aprovação; **restrita** é rejeitada por padrão; a origem e a licença de cada mídia obtida ficam registradas (“N asset(s) obtido(s) com origem e licença registradas”). **Geração de mídia por IA** vem **desligada** e, ligada, sempre pede aprovação e respeita o orçamento. A mídia adquirida vai para a pasta lateral `<projeto>-media/ai/` (durável — ver [backup](06-backup-e-recuperacao.md)).

## Limites reais desta versão

- A qualidade de resultado com **provedores reais** e a avaliação humana em demandas reais **não foram executadas** (ver [KNOWN_ISSUES](../KNOWN_ISSUES.md)); os testes usam um provedor determinístico.
- As fontes do Gateway são locais, URL aprovada ou catálogo de teste; não há integração com banco de stock real.
- Detecção de cenas é calibrada em material sintético; vídeo real com movimento de câmera pesado pode precisar de ajuste.

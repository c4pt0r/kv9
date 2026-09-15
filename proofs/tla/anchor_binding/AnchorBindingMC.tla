------------------------- MODULE AnchorBindingMC -------------------------
EXTENDS AnchorBinding
AllOK == [i \in 1..AMax |-> TRUE]
FirstBad == [i \in 1..AMax |-> i # 1]
=============================================================================

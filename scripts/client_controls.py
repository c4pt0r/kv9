"""Isolated mutations shared by client model and deductive checks."""

from ready_controls import replace_once


def mutations(model):
    dispatch = model[model.index("CRDispatch =="):model.index("CRRefuse ==")]
    reset = replace_once(dispatch, "crNow, crDeadline>>", "crNow>>")
    reset = replace_once(reset, "/\\ UNCHANGED", "/\\ crDeadline' = crNow + CRBudget\n    /\\ UNCHANGED")
    return [
        {"name": "retry-unknown-write", "property": "CRAtMostOneEffect",
         "model": replace_once(model,
             '/\\ crPhase = "ready" /\\ crSent = crClosed',
             '/\\ ((crPhase = "ready" /\\ crSent = crClosed) \\/ crPhase = "unknown")'),
         "proof_pattern": r"PROVE\s+CRDispatch => CRInvariant'"},
        {"name": "reset-logical-deadline", "property": "CRFixedBudget",
         "model": replace_once(model, dispatch, reset),
         "proof_pattern": r"PROVE\s+CRDispatch => CRInvariant'"},
        {"name": "refuse-applied-attempt", "property": "CRAtMostOneEffect",
         "model": replace_once(model,
             '/\\ crPhase = "pending" /\\ crApplied = {}',
             '/\\ crPhase = "pending" /\\ TRUE'),
         "proof_pattern": r"PROVE\s+CRRefuse => CRInvariant'"},
    ]

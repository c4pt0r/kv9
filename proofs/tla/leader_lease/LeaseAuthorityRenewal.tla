---------------------- MODULE LeaseAuthorityRenewal ----------------------
EXTENDS LeaseAuthorityMC
CONSTANT LAFollower
VARIABLE laStep

\* A reachability witness, not an exhaustive two-round safety configuration.
\* Every step must also be a legal action of the unchanged bounded protocol.
LARenewalNext ==
    /\ laStep < 16
    /\ LAMCNext
    /\ CASE laStep = 0 -> LAStartRound(1)
         [] laStep = 1 -> LAGrant(1, LALeader, LAGrantMin)
         [] laStep = 2 -> LAGrant(1, LAFollower, LAGrantMin)
         [] laStep = 3 -> LADeliver(1, LALeader)
         [] laStep = 4 -> LADeliver(1, LAFollower)
         [] laStep = 5 -> LAActivate(1)
         [] laStep = 6 -> LAReadBegin(1)
         [] laStep = 7 -> LAReadView(1)
         [] laStep = 8 -> LAReadFinish(1)
         [] laStep = 9 -> LAAdvance
         [] laStep = 10 -> LAStartRound(2)
         [] laStep = 11 -> LAGrant(2, LALeader, LAGrantMin)
         [] laStep = 12 -> LAGrant(2, LAFollower, LAGrantMin)
         [] laStep = 13 -> LADeliver(2, LALeader)
         [] laStep = 14 -> LADeliver(2, LAFollower)
         [] laStep = 15 -> LAActivate(2)
         [] OTHER -> FALSE
    /\ laStep' = laStep + 1

LARenewalSpec == (LAInit /\ laStep = 0) /\ [][LARenewalNext]_<<laVars, laStep>>
=============================================================================

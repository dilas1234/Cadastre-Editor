# Mutation Rollback: deferred (Phase 4)

**Status:** paused by PLAGIS decision, 2026-11-02. The rollback is not needed for now. It is kept
here so that it can be built later without having to research it again.

## Decision

PLAGIS considers Mutation Rollback unnecessary for now, because no rollback has been used since
2017. Phase 4 is complete without it. Nothing the rollback needs has been lost: the flag it reads
was imported, and so was the full lineage history.

## What Aumentum's rollback does

From the AUMENTUM CADASTRE Editor and Map User Guide, "Common Tasks":

- You select one parcel and right-click. The options are Parcel Mutation Separation, Parcel
  Mutation Split, **Mutation Rollback** and Open the Lineage Viewer. A job area is created
  automatically.
- "If you select Mutation Rollback, any parcel mutation that you have performed will be undone
  and the parcel will be set back to its original state before the mutation."
- "The Mutation Rollback function cannot be undone."

Configuration on PRODUCTIONMC:

- `ParcelAttributeTable.CanRollback` is a SmallInteger with domain `d_mutation_rollback`
  (1 Yes, 2 No) and default 1. It is editable, and `IsMutationInherited="True"`
  (ParcelAttributeTableDefinition.xml, ParcelAttributeConfig.xml).

## Evidence that rollback is unused

From the Phase 1 import of `sde.ParcelAttributeTable` (21,512 records, in
`cadastre.parcel_attribute`):

| CanRollback | Records |
|---|---|
| 1 (Yes) | 6 |
| NULL | 21,506 |
| 2 (No) | 0 |

All 6 records flagged 1 were created on 24–25 Nov 2014 (UPINs PL000000002490, 2491, 2504, 2507,
2533, 2534) and have no lineage. Aumentum's `dbo.parcelLineage` (237 rows, loaded into
`cadastre.parcel_lineage`) has no rollback: no mutation was reversed, and no historic parcel was
made active again.

The flag is carried on today. A new parcel from a mutation inherits `can_rollback` from its
parent (`routes/mutations.rs`, `inherit_attributes`), as `IsMutationInherited` requires.

## How to build it later

The new Cadastre already has what a rollback needs: the lineage rows (status 0 history and
status 1 active, mutation id, parents) and the retired shapes. Those shapes are still in
`cadastre.parcel` as `historic`/`archived` under their UPINs.

1. **Entry point.** Select one posted parcel, choose "Mutation Rollback" (Editor menu and lineage
   viewer), and give a workflow job. The job area is set to the mutation's footprint.
2. **Eligibility.** The parcel must be a child of the latest mutation in its lineage, and
   `CanRollback` must be 1 on every child of that mutation. No child may have been mutated
   again. **Registry rule:** no child may be linked to a Registry transaction that is Completed
   or Withdrawn (`registry.rs`, `check_link_allowed`). A UPIN with paid, completed transactions
   must never be retired by a rollback.
3. **Effect, applied at commit like any mutation:**
   - The mutation's children become `archived`, with archive reason "Mutation rollback".
   - The parents' last shapes go from `historic` back to `posted`, under their original UPINs.
     The "one posted parcel per UPIN" rule still holds, because the children have left.
   - The parents' attribute records are active again (archive date cleared). The children's
     records are archived.
   - Lineage: a rollback is recorded in the lineage itself, never by deleting rows. History
     must stay complete, so the original mutation's rows are kept.
4. **No undo.** A rollback cannot be rolled back, as in Aumentum. The UI says so before commit.
5. **UPINs.** The children's UPINs stay in `cadastre.issued_upin` and are never issued again.

## Open questions for when it is built

- How did Aumentum record a rollback in `parcelLineage`? There is no example in the data. It may
  have been an extra mutation type code, or rows being changed. Ask PLAGIS, or try it on a test
  Aumentum before building.
- Do Merge and Adjusted roll back the same way as Split? The guide doesn't say.
- Who may roll back? This one is answered already. The command is registered on PRODUCTIONMC as
  `Ils.MultiCadastre.Desktop.MutationRollbackCommand`, `{2C2D4007-7F56-4306-B54B-B7872AE93C21}`
  (`lrs-export/productionmc/com-commands.psv`). **No PLAGIS workflow step uses it**, neither in
  the captured `wf_step` commands nor in `job.xml`, so it could only be reached from the
  right-click menu. A rebuild should decide which job type and step it belongs to.

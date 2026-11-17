# Dmux guides

The documents in this folder are written for the people who use Dmux and for the people
who have to vouch for it. They avoid technical jargon; where a technical word is
unavoidable, it is explained in one line.

| Document | Who it is for | What it answers |
| --- | --- | --- |
| [How Dmux decides](../how-dmux-decides.md) | Bank staff and auditors who read and sign the report | What a fingerprint is, why row order does not matter, what each verdict means, what a PASS does not prove, and the md5 limit. |
| [Dmux for the database administrator](dba-setup.md) | The DBA on the old and new servers | The read-only login, why a superuser is refused, certificate trust, poolers, freeze window or replica advice, load, and how to watch or cancel a run. |
| [Dmux user guide](user-guide.md) | The person running the comparison | Installing Dmux, then the seven screens step by step: Connect, Compare structure, Map, Choose checks, Run, Results, Report. |
| [Verifying a Dmux download](verify-download.md) | Anyone who downloaded Dmux | How to check that an installer or CLI download really came from this project, using the checksums, the signature and the build provenance. |

Related material, kept where the work happened:

- [PLAN.md](../PLAN.md): the full specification, including the three levels of assurance
  and the safety rules.
- [DECISIONS.md](../DECISIONS.md): each design decision in two lines, with the reason.
- [`docs/dba/create_dmux_reader.sql`](../dba/create_dmux_reader.sql): the script that
  creates the SELECT-only login.
- [testdata/phase0/RESULTS.md](../../testdata/phase0/RESULTS.md): the proof that identical
  data gives identical fingerprints on PostgreSQL 9.4 to 18, and that planted changes are
  caught.

> **Status.** The method document and the administrator guide describe behaviour that is
> implemented and tested in the engine. The user guide is a skeleton: the screens are
> being wired to the engine, and the **TODO** notes in it mark the screenshots and labels
> still to be confirmed. Anything that changes a claim in these documents should change
> [PLAN.md](../PLAN.md) first.

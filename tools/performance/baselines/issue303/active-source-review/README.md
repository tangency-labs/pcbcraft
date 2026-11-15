# Active-source clarity

Following @travle1's confusion over which rules apply, `0cca4c2` separates class
contributions, saved effective values and unsaved assignment previews. Unspecified
class fields display Not specified instead of borrowing a baseline number that
could look authored. The effective page names only sources defining the current
bound or valid preference, with inactive candidates explained in Details.

A read-only helper over the existing Resolution distinguishes Active, Overridden,
LessRestrictive and Conflicting contributions. Explicit class preferences override
the baseline even when their numeric values are identical. Equal hard limits can
jointly define a bound; weaker limits do not determine its effective value. Null
preferences have no active winner. The existing resolution/validation policy,
source values, IDs/revisions/order and serialization remain unchanged.

Three engine regressions cover those precedence/limit cases, unresolved inherited
and competing preferences, and an addressed board maximum. A real-control test
checks source names and snapshot identity before assignment, effective provenance
after assignment, and overridden details. Existing tests also assert that unset
class fields remain disabled and read Not specified. The full workspace passed
1,862 tests with 11 ignored, alongside Clippy, the 15-test harness, browser checking/
packaging and native app/CLI builds. [Verification](verification.json) records scope,
commands, source fingerprints and limits.

Native GPU captures inspect all four states in wide dark and compact light hosts.
The packaged Chrome148 WebGL2/SwiftShader smoke additionally exercises active and
preview column headings, restoring the saved selection without applying, resize,
routing preference inspection and saved origins. No physical presentation, timing
or endurance result is inferred. Raw readbacks, logs and the isolated temporary
render helper remain under `setup/issue303/active-source-review` in the development
workspace. No worker, graph/cache, saved vocabulary or dependency was introduced.

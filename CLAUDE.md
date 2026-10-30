# lvpm

## Slices, issues and the board

Every slice is an issue under #57, shown on the
[lvpm mission](https://github.com/users/ThatsTheEnd/projects/2) board. The issue
is the work item; the board only adds its Status. How to run a slice is in
[docs/vision/build-slices.md](docs/vision/build-slices.md#how-to-run-a-slice).

When you start a slice:

1. Set its card to **In Progress** (`N` is the issue number):

   ```bash
   item=$(gh project item-list 2 --owner ThatsTheEnd --limit 300 --format json \
     -q ".items[] | select(.content.number==N) | .id")
   gh project item-edit --project-id PVT_kwHOA_lWOM4Bo99Q --id "$item" \
     --field-id PVTSSF_lAHOA_lWOM4Bo99QzhnboIQ --single-select-option-id 47fc9ee4
   ```

2. The PR body says `Closes #N`. Merging it closes the issue and the board
   moves the card to Done; never close a slice issue or set Done by hand.
3. A slice built differently from its issue gets the `deviation` label, and
   the PR says how.

Work that is not a slice needs no issue; its PR just doesn't say `Closes`.

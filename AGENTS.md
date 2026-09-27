# Commit reviews

Every commit in this project must be allowed to receive a roborev review.
Keep the installed roborev Git hooks enabled; do not bypass them, change the
hooks path to suppress them, or set skip/disable flags when committing.

After committing, verify that roborev queued a review for the new commit. If
the hook could not enqueue it, explicitly request a review of that commit with
roborev and report any remaining blocker. Do not describe a queued or running
review as completed.

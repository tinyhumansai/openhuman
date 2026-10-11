# Resource admission

Pure limits and sampled signals produce a resource-admission result. Explicit
modes win first, then server mode, AC requirements, severe CPU pressure and
normal/throttled thresholds. This utility performs no sampling, I/O, logging,
waiting or product configuration lookup.

Cron owns hardware sampling, config conversion, signed-out policy and the
cooperative wait lifecycle. This move reorganizes host utility ownership; it
does not remove a dependency or change scheduling behavior.

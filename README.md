# pupdate

simple helper utility to update remote systems alongside the local system easily. currently only pupdates through ssh+apt, your ssh key probably needs to be in ssh-agent for this to function properly.

## usage

run `pupdate -h` for help with arguments. with no arguments, pupdate will update the local system and any remotes configured in the config file (default ~/.pupdate).

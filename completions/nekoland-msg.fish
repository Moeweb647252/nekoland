# Print an optspec for argparse to handle cmd's options that are independent of any subcommand.
function __fish_nekoland_msg_global_optspecs
	string join \n h/help
end

function __fish_nekoland_msg_needs_command
	# Figure out if the current invocation already has a command.
	set -l cmd (commandline -opc)
	set -e cmd[1]
	argparse -s (__fish_nekoland_msg_global_optspecs) -- $cmd 2>/dev/null
	or return
	if set -q argv[1]
		# Also print the command, so this can be used to figure out what it is.
		echo $argv[1]
		return 1
	end
	return 0
end

function __fish_nekoland_msg_using_subcommand
	set -l cmd (__fish_nekoland_msg_needs_command)
	test -z "$cmd"
	and return 1
	contains -- $cmd[1] $argv
end

complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "query"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "window"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "popup"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "workspace"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "output"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "completion"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "subscribe"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "help"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "get_tree"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "get_outputs"
complete -c nekoland-msg -n "__fish_nekoland_msg_needs_command" -f -a "get_workspaces"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand query; and not __fish_seen_subcommand_from tree outputs workspaces" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand query; and not __fish_seen_subcommand_from tree outputs workspaces" -f -a "tree"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand query; and not __fish_seen_subcommand_from tree outputs workspaces" -f -a "outputs"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand query; and not __fish_seen_subcommand_from tree outputs workspaces" -f -a "workspaces"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand query; and __fish_seen_subcommand_from tree" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand query; and __fish_seen_subcommand_from outputs" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand query; and __fish_seen_subcommand_from workspaces" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and not __fish_seen_subcommand_from focus close move resize" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and not __fish_seen_subcommand_from focus close move resize" -f -a "focus"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and not __fish_seen_subcommand_from focus close move resize" -f -a "close"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and not __fish_seen_subcommand_from focus close move resize" -f -a "move"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and not __fish_seen_subcommand_from focus close move resize" -f -a "resize"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and __fish_seen_subcommand_from focus" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and __fish_seen_subcommand_from close" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and __fish_seen_subcommand_from move" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand window; and __fish_seen_subcommand_from resize" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand popup; and not __fish_seen_subcommand_from dismiss" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand popup; and not __fish_seen_subcommand_from dismiss" -f -a "dismiss"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand popup; and __fish_seen_subcommand_from dismiss" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand workspace; and not __fish_seen_subcommand_from switch create destroy" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand workspace; and not __fish_seen_subcommand_from switch create destroy" -f -a "switch"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand workspace; and not __fish_seen_subcommand_from switch create destroy" -f -a "create"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand workspace; and not __fish_seen_subcommand_from switch create destroy" -f -a "destroy"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand workspace; and __fish_seen_subcommand_from switch" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand workspace; and __fish_seen_subcommand_from create" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand workspace; and __fish_seen_subcommand_from destroy" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand output; and not __fish_seen_subcommand_from enable disable configure" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand output; and not __fish_seen_subcommand_from enable disable configure" -f -a "enable"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand output; and not __fish_seen_subcommand_from enable disable configure" -f -a "disable"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand output; and not __fish_seen_subcommand_from enable disable configure" -f -a "configure"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand output; and __fish_seen_subcommand_from enable" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand output; and __fish_seen_subcommand_from disable" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand output; and __fish_seen_subcommand_from configure" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand completion" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand subscribe" -l event -r
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand subscribe" -s h -l help
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand subscribe" -l json
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand subscribe" -l pretty
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand subscribe" -l jsonl
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand subscribe" -l no-payloads
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand help; and not __fish_seen_subcommand_from subscribe" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand help; and not __fish_seen_subcommand_from subscribe" -f -a "subscribe"
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand help; and __fish_seen_subcommand_from subscribe" -l json
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand help; and __fish_seen_subcommand_from subscribe" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand get_tree" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand get_outputs" -s h -l help -d 'Print help'
complete -c nekoland-msg -n "__fish_nekoland_msg_using_subcommand get_workspaces" -s h -l help -d 'Print help'

# winc 的 shell 集成（docs/TERMINAL.md §6.2）。winc 启动 zsh 时把 ZDOTDIR 指到这个目录，zsh 先读
# 这个文件：恢复用户原来的 ZDOTDIR，照常读用户自己的 .zshenv（之后的 .zprofile、.zshrc 也都按
# 用户的 ZDOTDIR 读），交互式的 shell 再挂上命令块的钩子。这个文件由 winc 生成，改了会被覆盖。

builtin typeset -g _winc_dir=${${(%):-%x}:A:h}
# terminal.prompt 关了时 winc 传 WINC_PROMPT=0：不换默认的提示符（见 winc.zsh）。
builtin typeset -g _winc_prompt=${WINC_PROMPT-1}
builtin unset WINC_PROMPT
# 报命令行时带上的随机数（docs/TERMINAL.md §6.3）：放在不导出的变量里，命令看不到。
builtin typeset -g _winc_nonce=${WINC_NONCE-}
builtin unset WINC_NONCE
if [[ -n ${WINC_ZDOTDIR+x} ]]; then
  builtin export ZDOTDIR=$WINC_ZDOTDIR
  builtin unset WINC_ZDOTDIR
else
  builtin unset ZDOTDIR
fi

{
  [[ -f ${ZDOTDIR:-$HOME}/.zshenv ]] && builtin source -- "${ZDOTDIR:-$HOME}/.zshenv"
} always {
  [[ -o interactive ]] && builtin source -- "$_winc_dir/winc.zsh"
  builtin unset _winc_dir
}

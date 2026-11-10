#!/usr/bin/env bash
# Run the AfriSovera regression REGISTRY and print a PASS/FAIL roll-up naming the
# failures, so "everything is green" is a claim this command makes rather than one
# a milestone note asserts.
#
# EVERY suite boots QEMU, so a full run is HOURS of wall-clock — this is a manual /
# CI runner, never something to start under a short (2-minute) tool timeout. It is
# built for that length:
#   * per-suite timeout, so a suite that hangs is recorded as TIMEOUT and the run
#     moves on instead of stalling the roll-up;
#   * results appended to a TSV AS EACH SUITE COMPLETES, so a run interrupted at
#     suite 400 still reports the 399 before it (--rollup / --resume read it back).
#     Each row is DATED with its completion time, so a reader can tell whether the
#     artifacts beside it are evidence about that row or about a later hand-rerun;
#     an undated row (written before this existed) is unknown, not agreement;
#   * FAILED distinguished from NEVER RAN — a registered script that is MISSING, or
#     that DIED before printing its own verdict, or that exits 0 with NO VERDICT at
#     all, is not a pass and is not lumped in with an honest failure. A suite
#     silently skipped is the hole that let two suites sit red for two milestones.
#   * the registry itself is audited against scripts/*-test.sh: a suite on disk that
#     nobody registered is UNREGISTERED and reds the roll-up, because it is exactly
#     the same hole wearing a green hat.
#   * EVIDENCE OUTLIVES THE RUN. Logs and serial captures used to go to the system
#     temp dir, which on a dev box is pruned by age while the run is still going:
#     measured on the 2h 52m full-registry run, the logs for the FIRST 276 of 673
#     suites (41%) were gone before it finished — a clean prefix, everything older
#     than a cutoff — which is why 24 of its 33 reds could not be triaged at all.
#     Both now go to a repo-local run directory (--logdir, default .rat-logs), and
#     the suite's own ${TMPDIR:-/tmp}/<x>.out capture lands there because TMPDIR is
#     exported into it. A run whose evidence something else may delete is a run that
#     has to be paid for twice.
#
# Usage:
#   scripts/run-all-tests.sh                     # full registry, fresh results file
#   scripts/run-all-tests.sh --only heap,smoke   # named subset (globs ok: 'wasm-*')
#   scripts/run-all-tests.sh --from heap         # registry order, starting at heap
#   scripts/run-all-tests.sh --resume            # skip suites already recorded
#   scripts/run-all-tests.sh --failed            # re-run only the non-PASS ones
#   scripts/run-all-tests.sh --rollup            # print the roll-up, run nothing
#   scripts/run-all-tests.sh --list              # registry audit + counts, run nothing
#   --timeout N (default 300s per suite)   --results FILE (default .rat-results.tsv)
#   --notrun-list N (default 12)          # how many NOT-RUN names to spell out
#   --logdir DIR (default .rat-logs)      # per-suite logs AND serial captures
#   --qemu-dir DIR ($QEMU_DIR)            # where qemu-system-aarch64 lives, if not on PATH
#
#   * QEMU IS RESOLVED ONCE, BEFORE THE FIRST SUITE. Every suite invokes
#     `qemu-system-aarch64` by bare name, so a shell without it on PATH does not
#     fail one suite — it fails ALL of them, identically, and WORSE than DIED:
#     measured, `ps-test.sh` in such a shell prints `command not found` and then
#     its own `PS TEST FAILED` line, so the runner classifies it FAIL — a verdict,
#     indistinguishable in the roll-up from an assertion that genuinely did not
#     hold. Whether a suite reads DIED or FAIL turns only on whether it happens to
#     print a verdict line, and neither is a fact about the kernel. The
#     Windows installer drops the binary in C:\Program Files\qemu and adds nothing
#     to PATH. It is found here, exported for every suite, and a run that cannot
#     find it REFUSES TO START rather than writing hundreds of identical DIED rows
#     nobody can tell apart from a real regression. --qemu-dir DIR / $QEMU_DIR
#     name the directory when it is somewhere else.
#
# Cleanup between suites is BY PID, keyed on this walk's own marker (2.887): kill_qemu
# hands afrish-walk-$AFRISH_WALK_ID to scripts/qemu-reap-2886.sh, which kills only the
# pids scripts/qemu-select-2881.sh printed as MATCH. The old machine-wide forms —
# `taskkill //IM` and `pkill -f qemu` — are GONE, so a guest this walk did not launch
# is no longer collateral. It still SAYS when it killed something, because a cleanup
# nobody can observe is the same hole as a suite that never prints a verdict.
set -uo pipefail
cd "$(dirname "$0")/.."

SUITES=(smoke smp pi dhcp tcp tcp-data tcp-close tcp-retransmit tcp-listen ifconfig persist persist-mtime config hash crc32 hmac bench rand rand-cap dice wasm-fuzz wasm-ops wasm-i64 wasm-i64mem wasm-i64loc wasm-f64 wasm-f32 wasm-glob wasm-memsz wasm-brtable editor-save uart-irq uart-rx-stress shell-wfi date ps df pmap stat gpio gpio-out reboot ls-l touch vfs-util hexdump grep wc find append virtio-rng base64 base32 uuid factor primes sort cpuinfo gcd timerinfo rev unhex pow caesar base smpinfo crc16 adler32 swapcase isqrt mmuinfo fnv1a gicinfo count popcount uartinfo ilog2 squeeze gray tcrinfo fib dedup digitsum histogram rtcinfo title isprime mairinfo tail ncr head rwords sumto mean minmax wrap cacheinfo roman nl lsvirtio collatz morse isar strings fwcfg totient nato tac mmfr divisors giccinfo atbash fletcher16 catalan uartflags rle hamming pfr vigenere icbrt dfr median acronym bitrev chars luhn dczinfo rot47 mode hex revnum cmp unique nthprime replace product gicid ascii weekday cut clidr ordinal perfect grepv isar1 pangram factorial grepn mmfr1 palindrome digitprod sortf pmcr anagram lucas line ccsidr slug primepi wordn ttbr vowels sumsq revf vbar soundex sumcubes avglen cpacr capitalize tribonacci linelen longestword pell maxline camel derangement grepc piglatin bell minline snake partition charcount pascal jacobsthal shortestword linewords padovan reveach perrin bytesum demorse sylvester lrc kebab doublefact inetsum unrle primorial mock superfact disemvowel leet armstrong grepi automorphic numeronym grepw happy railfence nextprime wordfreq boxed harshad bits triangular unbits isfib uniqwords despace pronic underline nextpow2 rot13 vowelcount wordlengths issquare letterfreq digitcount spaceout pentagonal maxword sortchars nextsquare minword double hexagonal firstword heptagonal lastword octagonal countchar squarefree shout charat semiprime countword sphenic uniqchars wordstartcount bigomega digitsonly omega wordendcount radical lettersonly smith alnum kaprekar vowelsonly addper consonantsonly evendigits swappairs isugly rot5 maxdigit strippunct istrimorphic rot18 isdisarium center iskeith indent ispernicious rjust ismersenne ljust isrepdigit zfill isdigitsorted repeat isundulating firstn isfactorion lastn ismunchausen trunc iszuckerman collapse isdudeney initials ispowerful middlechar ispalprime swapends isperfectpower sentencecase isevil dotcase isachilles sortbylen ispractical rotate tetrahedral interleave iscarmichael mirror pentatope isogram mobius chunk liouville traincase mertens countdistinct centeredhex rotwords starnumber censor repunit vowelcase thabit wordcountf cullen altwords emirp numberwords magicconstant sentencecap isgapful swapfirstlast wordsperline ispandigital lastletters istwinprime middleword charsperline isstrobogrammatic capitalizelast altdigitsum firstchars iscube doublewords lastchars fermat stutter isprimepower firstlast isbinpalindrome avgwordlen isbouncy swapwordpairs isarithmetic punctcount motzkin pluralize iscircularprime headlines ispolydivisible taillines uncamel hyperfactorial spacecount narayana syllables issafeprime wordvalue alternatingfactorial truncatewords woodall cardinal oddlines leyland evenlines wagstaff dedupwords iscousinprime tr issexyprime rwordsf centeredsquare firstunique centeredtriangular firstrepeat squarepyramidal constcase cototient countsubstr nonagonal casecount decagonal sortwordchars tetranacci col pentanacci wordat isharmonic wordlensf rotl wordvowels rotr rangelines hexanacci singularize centeredpentagonal rangewords centeredheptagonal revmiddle unwrap commaify indexof prevprime substr tailwords sumprimes stripprefix octahedral stripsuffix modinv before after askconv replacefirst jacobi between order nthfield npr insert iroot trim carmichael overlay stirling2 join asksqrt joinlines ilog delete zeckendorf fieldcount polygonal swapwords luhngen commonprefix hammingdist commonsuffix askprime repeateach digitsumbase longestrun ipow lastindexof centeredcube replacelast stellaoctangula startswith nextfib endswith compositorial everynth prevpow2 askfactor scrabble iscoprime lastwords sumfibs nthletter scrabblef nextcomposite wordindex nextpalindrome afterlast beforelast prevcomposite mask prevsquare repeatto prevfib insertword sumtotient deleteword askkb replaceword pisano nthchar divmod wordrange bitops askconv2 charrange sumrange repeatword nthdigit deletechar isqrtrem swapchar clamp askafricore reverseword sumdigitsquares upperword nextmultiple lowerword roundto capitalizeword prevmultiple duplicatewords askcube wordfromend sumdigitcubes linefromend maxprimefactor charfromend minprimefactor removeword nthdigitfromend nthvowel countdigit askgcd nthconsonant sumevendigits setword sumodddigits spelldigits sumprimefactors nthletterfromend sumdistinctprimefactors redact gcd3 swapcasef askconcept abbreviate lcm3 topword isqrtceil rareword rotdigits sortdesc nextcube askfib stripdigits prevcube everynthword nexttriangular countsentences prevtriangular countsentencesf nextpentagonal topchar askfactorial rarechar prevpentagonal headwords nexthexagonal titlef prevhexagonal acronymf nextheptagonal rarewordf askpower decapitalize prevheptagonal shoutf nextoctagonal whisperf prevoctagonal reveachf nextnonagonal consonantcase askdigitsum prevnonagonal mockf nextdecagonal rot13f ctz distinctwords parity atbashf crt2 askevenodd caesarf clz disemvowelf prevdecagonal punctonly primitiveroot leetf hendecagonal hapax askaverage nexthendecagonal vowelsonlyf prevhendecagonal hapaxf stern uppersonly dodecagonal lowersonly nextdodecagonal askmaxmin rot47f prevdodecagonal hapaxchars isprimitiveroot titlecount isself dotacronym pochhammer bacon askrange sqrtmod debacon dlog titlecountf lazycaterer unvigenere cake a1z26 frombase asksum askproduct una1z26 schroeder hapaxcharsf delannoy polybius lah unpolybius eulerian nthsentence stirling1 rotlines rotatef rot18f rotwordsf amicable issemiperfect isweird lastsentence firstsentence firstsentencef lastsentencef capitalizef decapitalizef sentencecasef nthsentencef nthvowelfromend tridecagonal tetradecagonal pentadecagonal hexadecagonal heptadecagonal octadecagonal enneadecagonal icosagonal henicosagonal board-rpi4 board-pine64 uart8250 board-check board-resolved board-machine gicbase sleep sleep-syscall fault heap frame-bounds stack-room alloc-bound reclaim-arith freelist-live heap-grow disk net net-rx-recycle tcp-serve tcp-serve2 tcp-ooo tcp-dest tcp-inuse peerip sgi-el0 audio speaktext longspeak rxsplit pc-boot pc-paging pc-ring3 pc-ipc pc-ahci pc-nvme pc-e1000 pc-usbkbd pc-ps2kbd pc-screen pc-userland pc-usbmouse pc-wmdrag pc-usbnet pc-usbhub pc-usbplug pc-usbstick pc-desktop pc-gpt pc-installplan pc-tablebackup pc-installpart pc-tablerestore pc-formatpart pc-formatsata pc-installdisk pc-installdisk-sata pc-bootentries pc-bootplan pc-bootcopy pc-bootentry pc-bootnext pc-fwinfo pc-wifi pc-ps2mouse pc-hda pc-igb pc-rtl pc-sdhci pc-powerbutton pc-xhci2 pc-xhci-multi pc-xhci-hotplug pc-usbhubplug pc-usbtablet pc-usbtether pc-usbaudio pc-usbserial pc-emmc pc-installdisk-emmc pc-sdbus pc-sdmulti pc-sdwrite pc-sddma desk pc-desk pc-help pc-report pc-screenfit pc-usbaccel pc-usbwheel pc-usbaddr pc-usbretry pc-usbsettle pc-usbsetaddr pc-usbdma pc-usbreset pc-usbports pc-usbpsi pc-usbmulti pc-usbbig pc-usb4k pc-rtcinfo pc-hdamic pc-nvmemulti pc-satamulti pc-ide pc-formatide pc-installdisk-ide pc-idemulti pc-bootcopy-ide)

TIMEOUT="${RAT_TIMEOUT:-300}"

# ---- PER-SUITE TIMEOUT OVERRIDE (3.340) -------------------------------------
# The cap is a HANG-CATCHER, not a grade on slowness. One suite costs more than
# the default allows: speaktext boots QEMU THREE times (two texts + the in-run
# control) and measured 281 s standalone, so in walk 3275 it was killed at the
# cap and recorded TIMEOUT rc=124 -- a red that says nothing about the property
# under test. Per 3.339's pre-registered remedy its assertions and its grader
# are NOT touched; it gets HEADROOM instead, and a per-suite override is used
# rather than a blanket RAT_TIMEOUT=600 so the other 686 suites keep the tight
# cap that catches THEIR hangs.
#
# The number is NOT a predicted contention factor. 3.116 killed both the
# multiplicative and the additive walk-cost model, so stamping a factor here
# would be re-running a dead model. It is picked as a hang-catcher: clear of the
# measured standalone cost, still finite, so a suite that genuinely HANGS is
# still killed and still recorded as TIMEOUT.
#
# An override may only RAISE a suite's cap, never lower it, and an explicit
# --timeout / RAT_TIMEOUT above the override still wins -- the operator can
# always widen the whole walk without editing this table.
SUITE_TIMEOUT_OVERRIDES=(speaktext)
suite_timeout() {
    local t
    case "$1" in
        speaktext) t="${RAT_TIMEOUT_SPEAKTEXT:-900}" ;;
        *)         t="$TIMEOUT" ;;
    esac
    if [ "$t" -gt "$TIMEOUT" ]; then printf '%s' "$t"; else printf '%s' "$TIMEOUT"; fi
}

RESULTS="${RAT_RESULTS:-.rat-results.tsv}"
LOGDIR="${RAT_LOGDIR:-.rat-logs}"      # repo-local: the system temp is pruned mid-run
NOTRUN_LIST="${RAT_NOTRUN_LIST:-12}"   # how many NOT-RUN names the roll-up spells out
QEMU="${RAT_QEMU:-qemu-system-aarch64}"  # overridable so the not-found path is testable
QEMU_DIR="${QEMU_DIR:-}"               # explicit override; searched first, never guessed past
MODE=run           # run | rollup | list
SELECT=all         # all | only | from | failed | resume
ONLY=""
FROM=""

while [ $# -gt 0 ]; do
    case "$1" in
        --timeout) TIMEOUT="$2"; shift 2 ;;
        --results) RESULTS="$2"; shift 2 ;;
        --only)    SELECT=only; ONLY="$2"; shift 2 ;;
        --from)    SELECT=from; FROM="$2"; shift 2 ;;
        --resume)  SELECT=resume; shift ;;
        --failed)  SELECT=failed; shift ;;
        --rollup)  MODE=rollup; shift ;;
        --notrun-list) NOTRUN_LIST="$2"; shift 2 ;;
        --logdir)  LOGDIR="$2"; shift 2 ;;
        --qemu-dir) QEMU_DIR="$2"; shift 2 ;;
        --list)    MODE=list; shift ;;
        -h|--help) sed -n "2,$(( $(grep -n '^set -uo pipefail' "$0" | head -1 | cut -d: -f1) - 1 ))p" "$0" |
                   sed 's/^# \?//'; exit 0 ;;
        *) echo "[run-all] unknown argument: $1 (try --help)" >&2; exit 2 ;;
    esac
done

# ---- registry audit: a suite on disk that nobody registered never runs at all ----
declare -A REGISTERED
for s in "${SUITES[@]}"; do REGISTERED[$s]=1; done
UNREGISTERED=()
for f in scripts/*-test.sh; do
    # 2.860: parameter expansion, NOT `basename`. This loop runs once per file in
    # scripts/*-test.sh at EVERY start (684 of them), and each `$(basename ...)`
    # is a fork+exec: measured 94-106 s of pure startup latency before the runner
    # reaches its own guard. `${f##*/}` is the same string with no process at all.
    b="${f##*/}"; b="${b%-test.sh}"
    [ -n "${REGISTERED[$b]:-}" ] || UNREGISTERED+=("$b")
done

# ---- results file: one TSV row per completed suite, appended as it completes ----
# columns: suite <TAB> status <TAB> seconds <TAB> exit-code <TAB> completed-at (epoch)
#
# The fifth column exists so a row can be dated. Without it a row is a verdict with
# no time on it, and nothing downstream can tell whether the artifacts sitting next
# to it are evidence about THIS row or about a hand-rerun that happened afterwards —
# a suite re-run alone leaves a log newer than the row, and triage would happily
# classify the row from it. Rows written before this column existed have no time and
# are read back as `?`: unknown, which readers must treat as unknown rather than as
# agreement, because "I cannot check" is not "I checked and it matched".
declare -A RECORDED
load_results() {
    RECORDED=()
    [ -f "$RESULTS" ] || return 0
    while IFS=$'\t' read -r s st sec rc ts; do
        [ -n "${s:-}" ] || continue
        RECORDED[$s]="${st}"$'\t'"${sec:-?}"$'\t'"${rc:-?}"$'\t'"${ts:-?}"  # last row wins
    done < "$RESULTS"
}
load_results

matches_only() {
    local s="$1" pat
    IFS=',' read -ra pats <<< "$ONLY"
    for pat in "${pats[@]}"; do
        # shellcheck disable=SC2053
        [[ "$s" == $pat ]] && return 0
    done
    return 1
}

# ---- which suites this invocation will actually run ----
TORUN=()
started=0
for s in "${SUITES[@]}"; do
    case "$SELECT" in
        all)    TORUN+=("$s") ;;
        only)   matches_only "$s" && TORUN+=("$s") ;;
        from)   [ "$s" = "$FROM" ] && started=1; [ "$started" = 1 ] && TORUN+=("$s") ;;
        resume) [ -z "${RECORDED[$s]:-}" ] && TORUN+=("$s") ;;
        failed) [ -n "${RECORDED[$s]:-}" ] && [ "${RECORDED[$s]%%$'\t'*}" != PASS ] && TORUN+=("$s") ;;
    esac
done
[ "$SELECT" = from ] && [ "$started" = 0 ] && { echo "[run-all] --from: no such suite: $FROM" >&2; exit 2; }
[ "$SELECT" = only ] && [ "${#TORUN[@]}" = 0 ] && { echo "[run-all] --only: matched no suite: $ONLY" >&2; exit 2; }

rollup() {
    local total_reg=${#SUITES[@]} s st sec rc ts row
    local pass=0 other=0
    local -a failures=() notrun=()
    echo
    echo "=============== AfriSovera regression roll-up (${RESULTS}) ==============="
    for s in "${SUITES[@]}"; do
        row="${RECORDED[$s]:-}"
        # NOT-RUN is kept apart from the verdicts: after `--only heap` it is 672 suites
        # long and would bury the one line that matters. It is still counted, never green.
        if [ -z "$row" ]; then other=$((other + 1)); notrun+=("$s"); continue; fi
        IFS=$'\t' read -r st sec rc ts <<< "$row"
        if [ "$st" = PASS ]; then pass=$((pass + 1));
        else other=$((other + 1)); failures+=("$(printf '%-22s %-10s rc=%-4s %ss' "$s" "$st" "$rc" "$sec")"); fi
    done
    if [ "${#failures[@]}" -gt 0 ]; then
        echo "  NOT GREEN (${#failures[@]} reached a verdict and lost, or died trying):"
        printf '    %s\n' "${failures[@]}"
    fi
    if [ "${#notrun[@]}" -gt 0 ]; then
        echo "  NOT RUN (${#notrun[@]} registered, no result in ${RESULTS} — not a pass):"
        printf '    %s\n' "${notrun[@]:0:$NOTRUN_LIST}"
        [ "${#notrun[@]}" -gt "$NOTRUN_LIST" ] &&
            echo "    ... and $(( ${#notrun[@]} - NOTRUN_LIST )) more (--notrun-list N to see them; --resume runs them)"
    fi
    if [ "${#failures[@]}" -gt 0 ] || [ "${#notrun[@]}" -gt 0 ]; then
        echo "  -----------------------------------------------------------------"
        echo "  FAIL/TIMEOUT = ran and lost.  MISSING/DIED/NO-VERDICT/NOT-RUN = never"
        echo "  reached a verdict — that is not a pass, and it is how a red suite hides."
    fi
    if [ "${#UNREGISTERED[@]}" -gt 0 ]; then
        echo "  UNREGISTERED (on disk, in no registry — never runs at all): ${#UNREGISTERED[@]}"
        printf '    %s-test.sh\n' "${UNREGISTERED[@]}"
    fi
    echo "  -----------------------------------------------------------------------"
    echo "  ${pass}/${total_reg} registered suites PASS; ${other} not green; ${#UNREGISTERED[@]} unregistered"
    echo "  per-suite logs: ${LOGDIR}/rat-<suite>.log"
    echo "========================================================================"
    [ "$other" -eq 0 ] && [ "${#UNREGISTERED[@]}" -eq 0 ]
}

if [ "$MODE" = list ]; then
    echo "[run-all] ${#SUITES[@]} suites registered; ${#UNREGISTERED[@]} on disk unregistered; this selection would run ${#TORUN[@]}"
    [ "${#UNREGISTERED[@]}" -gt 0 ] && printf '  UNREGISTERED: %s-test.sh\n' "${UNREGISTERED[@]}"
    # the table is READ back through suite_timeout, never restated, so a cap
    # printed here cannot disagree with the cap the run actually applies.
    for _s in "${SUITE_TIMEOUT_OVERRIDES[@]}"; do
        printf '  PER-SUITE CAP: %s %ss (global %ss)\n' "$_s" "$(suite_timeout "$_s")" "$TIMEOUT"
    done
    printf '  %s\n' "${TORUN[@]}"
    exit 0
fi
if [ "$MODE" = rollup ]; then rollup; exit $?; fi

# ---- QEMU, found ONCE, or the run does not start ----
# A missing PATH entry is not a test result. Every registered suite runs QEMU by bare
# name, so in a shell that cannot see it all of them die the same death within seconds
# of each other and the roll-up reports the whole tree red — a shape that has cost real
# triage time and says nothing about the kernel. Found here and exported, one lookup
# covers every suite without editing any of them; not found here, the run refuses.
find_qemu() {
    command -v "$QEMU" >/dev/null 2>&1 && { QEMU_AT="$(command -v "$QEMU")"; return 0; }
    local d
    for d in ${QEMU_DIR:+"$QEMU_DIR"} \
             "/c/Program Files/qemu" "/c/Program Files (x86)/qemu" \
             "${HOME}/scoop/apps/qemu/current" \
             /usr/local/bin /opt/homebrew/bin /usr/bin /usr/local/opt/qemu/bin; do
        [ -d "$d" ] || continue
        if   [ -x "${d}/${QEMU}" ];     then QEMU_AT="${d}/${QEMU}"
        elif [ -x "${d}/${QEMU}.exe" ]; then QEMU_AT="${d}/${QEMU}.exe"
        else continue; fi
        PATH="${d}:${PATH}"; export PATH   # exported: every suite inherits the lookup
        return 0
    done
    return 1
}
QEMU_AT=""
if find_qemu; then
    echo "[run-all] qemu: ${QEMU_AT}"
else
    {
        echo "[run-all] ${QEMU} is not on PATH, and is not in any of the usual install"
        echo "[run-all] directories. REFUSING TO START: without it every one of the"
        echo "[run-all] ${#TORUN[@]} selected suites would die identically and be recorded"
        echo "[run-all] as FAIL or DIED — a red registry that says nothing about the kernel."
        echo "[run-all] Fix: --qemu-dir DIR (or QEMU_DIR=DIR), or put it on PATH."
        echo "[run-all]   Windows: C:\\Program Files\\qemu    Debian: apt install qemu-system-arm"
    } >&2
    exit 2
fi

# ---- where the evidence goes, and why it is not the system temp ----
# A full run is hours long, and the system temp dir is pruned BY AGE while it runs:
# measured on the 2h 52m registry run, the logs for suites 1..276 of 673 were gone
# before it ended and 277..673 were all present — a clean prefix, i.e. a cutoff, not
# a write that failed. Evidence a run cannot keep is evidence the next triage does
# not have. Both artifacts now go to a repo-local directory: the runner's own log
# directly, and the suite's serial capture because every suite names it
# ${TMPDIR:-/tmp}/<x>.out and TMPDIR is exported to point here (checked: 0 of the
# registered suites hardcode /tmp, so this redirects all of them without editing any).
mkdir -p "$LOGDIR" || { echo "[run-all] cannot create log dir: $LOGDIR" >&2; exit 2; }
LOGDIR="$(cd "$LOGDIR" && pwd)"          # absolute: suites cd to their own dirs
export TMPDIR="$LOGDIR"
echo "[run-all] evidence -> ${LOGDIR} (logs + serial captures; survives the run)"

# ---- the walk marker: every guest this walk launches says so in its own argv ----
# kill_qemu's `taskkill //IM qemu-system-aarch64.exe` is machine-wide: it kills EVERY
# emulator on the box, including a second walk's and a colleague's, and 2.881 measured
# it killing a renamed non-VM too — the blast radius is the image NAME, not the process.
# taskkill has no COMMANDLINE filter (measured off `taskkill /?` on this host), so the
# fix needs two halves: mint a per-walk marker into the guest's argv (here), then select
# on it via CIM and kill BY PID (scripts/qemu-select-2881.sh). This is the mint half.
#
# Mechanism, measured by scripts/mint-probe-2882.sh (6/6, all arms reached): a shim
# DIRECTORY prepended to PATH. 2.882's census says 684 suites call qemu by BARE NAME
# and 0 reference $QEMU_AT or any path separator, so the command word is a bare name in
# both resolution modes and the shim wins in both — including --qemu-dir mode, whose
# handling at :220 also only prepends a directory (which is why 2.880's "--qemu-dir makes
# it an absolute path" hole is not real). ORDERING IS LOAD-BEARING and this placement is
# the whole reason it works: AFTER find_qemu, so the shim shadows the real binary rather
# than being shadowed by it (probe ARM 4), and after LOGDIR is absolute, so the shim dir
# lives with the rest of the walk's evidence. The shim REFUSES (exit 3) on a blank or
# non-[A-Za-z0-9] id rather than minting a blank marker, because a blank marker is the
# wildcard qemu-select-2881.sh already refuses to treat as "all guests".
AFRISH_WALK_ID="${AFRISH_WALK_ID:-$(date -u +%Y%m%d%H%M%S)$$}"
case "$AFRISH_WALK_ID" in
    ''|*[!A-Za-z0-9]*) echo "[run-all] AFRISH_WALK_ID must be [A-Za-z0-9]+, got '$AFRISH_WALK_ID'" >&2; exit 2 ;;
esac
export AFRISH_WALK_ID QEMU_AT
QEMU_SHIM_DIR="${LOGDIR}/qemu-shim"
mkdir -p "$QEMU_SHIM_DIR" || { echo "[run-all] cannot create shim dir: $QEMU_SHIM_DIR" >&2; exit 2; }
# 2.872: the shim also MINTS THE GUEST'S WINDOWS PID into pids/, so kill_qemu can taskkill by
# pid without paying for a CIM selector (2.867 measured that selector at 26.6 s of real work
# per call).  Minting requires the child to be a BACKGROUND job — /proc/$!/winpid does not
# exist for a process you exec'd into — so the shim becomes TWO processes where it was one.
# That is the whole risk, and scripts/shimpid-probe-2872.sh graded it 7/7 on a real boot
# before this was installed: the trap is what keeps `timeout N qemu` honest (ARM6 — timeout
# kills the shim, the trap forwards it, the real guest is dead 1.5 s later), and ARMS 4/5/7
# hold exit-code, stdin, and the old exec shape (still taken verbatim when QEMU_PIDS_DIR is
# unset, so a suite that runs the shim outside a walk keeps the single-process behaviour).
# 2.871 is why the minted pid is written under its own name and not called a stub: on the
# REAL qemu binary the settle loop converges on the guest's own Windows pid (ARM2 here
# re-proved it, minted == the pid a CIM CommandLine match found independently).
QEMU_PIDS_DIR="${QEMU_SHIM_DIR}/pids"
mkdir -p "$QEMU_PIDS_DIR" || { echo "[run-all] cannot create pids dir: $QEMU_PIDS_DIR" >&2; exit 2; }
export QEMU_PIDS_DIR
cat > "${QEMU_SHIM_DIR}/${QEMU}" <<'SHIM_EOF'
#!/bin/sh
# minted by run-all-tests.sh: tag this walk's guests so they can be killed BY PID
case "${AFRISH_WALK_ID:-}" in
    ''|*[!A-Za-z0-9]*) echo "qemu shim: refusing, AFRISH_WALK_ID unset or not [A-Za-z0-9]+" >&2; exit 3 ;;
esac
[ -x "${QEMU_AT:-}" ] || { echo "qemu shim: QEMU_AT not executable" >&2; exit 3; }
# no pids dir -> the 2.885 shape verbatim: one process, exec, nothing to go wrong
[ -d "${QEMU_PIDS_DIR:-}" ] || exec "$QEMU_AT" -name "afrish-walk-${AFRISH_WALK_ID}" "$@"
# pids dir -> background + /proc/$!/winpid (2.871: on the real binary this pid IS the guest)
exec 3<&0
"$QEMU_AT" -name "afrish-walk-${AFRISH_WALK_ID}" "$@" 0<&3 &
qpid=$!
killed=0
trap 'killed=1; kill "$qpid" 2>/dev/null' TERM INT HUP
a=""; b=""; i=0
while [ $i -lt 40 ]; do
    b="$(cat /proc/$qpid/winpid 2>/dev/null)"
    [ -n "$b" ] && [ "$b" = "$a" ] && break
    a="$b"; i=$((i+1)); sleep 0.1
done
[ -n "$a" ] && echo "$a" > "${QEMU_PIDS_DIR}/$a" 2>/dev/null
wait "$qpid"; rc=$?
# on the trap path the guest may have OUTLIVED us, so the pid file STAYS (that leftover is
# exactly what kill_qemu is for).  On the normal path the child is reaped, so drop it rather
# than leave a stale pid that a recycled Windows pid could make lethal.
if [ "$killed" = 1 ]; then wait "$qpid" 2>/dev/null; else [ -n "$a" ] && rm -f "${QEMU_PIDS_DIR}/$a"; fi
exit $rc
SHIM_EOF
chmod +x "${QEMU_SHIM_DIR}/${QEMU}"
PATH="${QEMU_SHIM_DIR}:${PATH}"; export PATH
echo "[run-all] walk marker: afrish-walk-${AFRISH_WALK_ID} (shim ${QEMU_SHIM_DIR}, pids ${QEMU_PIDS_DIR})"

# 2.859: THE RUNNER REFUSES TO BE THE SECOND WRITER.
#
# 2.858 measured what prose could not prevent: EIGHTEEN live run-all-tests.sh, with
# eight different argument shapes, all appending to ONE .rat-results.2948.tsv (488
# rows / 260 distinct / 40 suite names recording BOTH a PASS and a FAIL). Six
# consecutive dispatches each armed a runner believing it was the only one, and the
# gate armed on that file had no check that could see a second writer -- it would
# have graded interleaved rows as confidently as clean ones.
#
# The handle already carried everything needed to stop this from 2.792 onward
# (results=, pid=, winpid=, startedZ=); NOTHING read it at startup. This does, and
# it runs BEFORE the truncation below -- a refused runner must not even blank the
# file the live walk is appending to, let alone add a row to it.
#
# WHAT IT GRADES, three relations, no typed-in pid:
#   1. a handle exists beside THIS results file and its WRITER row names it
#      (a handle about another file is not ours to read -- silently proceed);
#   2. it carries NO EXIT row (the 2.858 trap stamps one, so a closed handle is a
#      walk that has ENDED and cannot be collided with);
#   3. its winpid is LIVE, asked of `arm-holders.sh --live`, which is the tasklist
#      form scripts/liveprobe-falsify-2831.sh falsified (2.765 rule 5: CALL the
#      probe that exists, never re-type it).
#
# UNKNOWN IS NOT DEAD, and here it is not GO either: --live exit 37 (the probe
# errored) refuses too. The flattering direction -- "could not tell, so start
# anyway" -- is the exact one that cost six dispatches. RAT_ALLOW_CONCURRENT=1 is
# the deliberate override for the rare genuine parallel run into DISJOINT results
# files; it is a typed opt-in, which is what a second writer never was.
# The guard itself lives in scripts/writer-guard.sh and is CALLED UNEDITED (2.765
# rule 5). It is a separate file for a measured reason, recorded in its header: as
# a function in here it could only be exercised by starting the whole runner, and
# the runner spends ~60 s registering 684 suites before this line is reached -- the
# first selftest's 12 s leash never got here, so all seven of its arms were vacuous
# and read as green. The refusal must exit THIS process, so the call is graded here.
#
# ORDER IS LOAD-BEARING, twice over: this runs BEFORE the truncation below, so a
# refused runner cannot blank the file a live walk is appending to; and BEFORE the
# EXIT trap is installed further down, so a refusal cannot stamp an EXIT row on
# that walk's handle -- which would hand the NEXT runner a false all-clear.
if [ -f scripts/writer-guard.sh ]; then
    if ! guard_out="$(bash scripts/writer-guard.sh "$RESULTS" 2>&1)"; then
        printf '%s\n' "$guard_out" | sed 's/^/[run-all] /' >&2
        echo "[run-all] this process has written no row and will write none." >&2
        exit 88
    fi
    echo "[run-all] guard: $(printf '%s\n' "$guard_out" | head -1)"
fi

# A fresh full run starts a fresh results file; every selective run APPENDS, so an
# interrupted run's rows survive the --resume / --failed that continues it.
[ "$SELECT" = all ] && : > "$RESULTS"

# 2.858: THE WALK CLOSES ITS OWN HANDLE. Measured on the real runner: a walk that
# exits cleanly leaves a handle with no EXIT row, so handle-check.sh answers
# HANDLE-INFLIGHT (5) forever and close-out.sh refuses (15) until a human runs
# `stamp-handle.sh --exit` by hand. That script CANNOT be the fix from in here:
# called from a trap it refuses 52 (the winpid it probes is its own live parent)
# and 53 (no holder has observed an exit yet). The writer is the one observer that
# knows its own rc WHILE STILL ALIVE, so it writes its own row.
#
# It DERIVES NOTHING it could get wrong, and refuses rather than lie:
#   * no handle, or no WRITER row, or a WRITER row about a DIFFERENT results file
#     -> silent no-op (a launcher that never stamped gets no invented handle);
#   * an EXIT row already present -> left alone (a closed walk is closed once);
#   * rows= is `wc -l` of the results file at exit, which is what handle-check
#     grades EXACTLY on an EXIT row -- not a counter this loop maintains;
#   * the WRITER row is carried through byte-for-byte, exactly as
#     stamp-handle.sh --exit does, so the two shapes stay interchangeable.
# The trap never changes the runner's exit code: rollup's verdict is still the rc.
#
# 3.666 (queue item 1b, the PRODUCER half of 3.663's own residual gap):
# `detail=rc=$?` is whatever the last foreground command happened to return, never
# a record of the signal itself -- which is exactly why a kill at row N>0 that
# leaves `$?=0` (3.660/3.663's SIGTERM shapes) stamps an EXIT row indistinguishable
# from a genuine completed walk: both read rows=N, rc=0. AFRISH_SIGNALED is read
# here, not derived -- the trap below is the only thing that ever writes it.
#
# MEASURED, not assumed: the first attempt folded the signal INTO detail= as
# `rc=0,signal=TERM`, and a REAL run against it broke handle-check.sh's own
# `sed -n 's/^rc=\([0-9][0-9]*\)$/\1/p'` -- an EXACT, anchored match against the
# whole detail= string -- turning a healthy-looking-but-signalled row into
# HANDLE-MALFORMED (9) instead of the unflattering-but-correct HANDLE-GREEN (0)
# it reads as today. That is a CONSUMER-side behaviour change from a
# PRODUCER-only tick, exactly the boundary this queue item is scoped not to
# cross. signal= is instead its own tab-delimited field on the EXIT row, same
# shape as WRITER's own multi-field row (results=/pid=/winpid=/startedZ=) --
# handle-check.sh's field() picks fields OUT BY NAME, so an unread field it
# never asked for is invisible to it, and detail= stays byte-for-byte `rc=N` as
# it always has. Re-verified after the fix: same real artefact, handle-check.sh
# now reads it back as HANDLE-GREEN again, unchanged from before this tick.
AFRISH_SIGNALED=""
selfclose_handle() {
    local rc="$1" hf="${RESULTS}.handle" wrow hbase base frows sigfield
    [ -f "$hf" ] || return 0
    wrow="$(grep -E '^WRITER'$'\t' "$hf" 2>/dev/null | tr -d '\r' | tail -1)"
    [ -n "${wrow:-}" ] || return 0
    hbase="$(printf '%s\n' "$wrow" | tr '\t' '\n' | grep -m1 '^results=' | cut -d= -f2-)"
    base="$(basename "$RESULTS")"
    [ "${hbase:-}" = "$base" ] || return 0
    if grep -qE '^EXIT'$'\t' "$hf" 2>/dev/null; then return 0; fi
    if [ -f "$RESULTS" ]; then frows="$(wc -l < "$RESULTS" | tr -d ' ')"; else frows=0; fi
    sigfield=""
    [ -n "${AFRISH_SIGNALED:-}" ] && sigfield="$(printf '\tsignal=%s' "$AFRISH_SIGNALED")"
    printf '%s\n' "$wrow" > "$hf"
    printf 'EXIT\trows=%s\tdetail=rc=%s%s\tholders=self\n' "$frows" "$rc" "$sigfield" >> "$hf"
    echo "[run-all] closed own handle ${hf}: EXIT rows=${frows} rc=${rc}${sigfield}"
}
trap 'selfclose_handle $?' EXIT

# 3.666: the MAIN process, not the qemu-shim child, now carries its own TERM/HUP
# trap (NOT INT -- see below). Measured by 3.664 (`grep -n '^trap' scripts/run-all-tests.sh`):
# before this, the only two traps in the file were line ~350's, INSIDE the qemu-shim
# heredoc (a DIFFERENT process, the minted guest wrapper), and this file's own EXIT
# trap above -- the outer runner had no signal trap of its own at all. SIGKILL still
# cannot be caught by any shell (no producer fix reaches it, by design). TERM and HUP
# are the two signals 3.660/3.664 measured actually KILLING the real runner (mid-suite,
# `rows=0` today) -- those are exactly the kills queue item (1b) needs stamped.
# `afrish_note_signal` records which one arrived, resets the trap to default for that
# signal, then RE-RAISES the SAME signal at itself: the process still dies exactly the
# way 3.660/3.664 measured it dying before (default disposition, EXIT trap still fires
# unchanged), so this is additive for TERM/HUP -- it changes what gets WRITTEN, never
# whether or how the walk dies.
#
# INT IS DELIBERATELY LEFT UNTRAPPED. TOOL_REGISTRY's "A HEALTH GRADE READ OFF A FIELD
# THAT CARRIES NO HEALTH SIGNAL" entry (3.661) records SIGINT as DISCARDED ENTIRELY by
# the real runner -- byte-for-byte the unsignalled control -- because bash defers an
# UNTRAPPED signal while blocked in a foreground child and, for INT specifically, never
# acts on it at all here. Installing an explicit trap changes that: bash defers a
# TRAPPED signal only until the foreground command completes, then RUNS the trap -- so
# a runner that currently survives SIGINT unscathed would start dying one command late,
# a real behaviour change to the walk queue item (1b) is scoped not to make (same
# consumer/producer boundary handle-check.sh:106-109 already drew: teaching the runner
# to trap a signal it doesn't already die to is a bigger blast radius than stamping the
# ones it does). That's a prediction, not yet falsified against the real runner -- left
# for a follow-up tick, not asserted here.
afrish_note_signal() {
    AFRISH_SIGNALED="$1"
    trap - TERM HUP
    kill -s "$1" $$
}
trap 'afrish_note_signal TERM' TERM
trap 'afrish_note_signal HUP' HUP

# 2.863: THE WALK ARMS ITS OWN HANDLE AND ITS OWN KEEPER.
# 2.862 shipped keeper-launch.sh and closed with the gap that its own launch had
# only moved up one level: an AGENT still had to remember to run it beside a
# walk, and 2.836's header still reads true -- `grep -n handle run-all-tests.sh`
# found only the close-out half, so NOTHING in the tree wrote the handle either.
# The walk is the one process that always knows $RESULTS and is always running
# when the handle is needed, so it does both here, in the same place, and the
# 2.858 trap above closes what this opens.
#
# It DERIVES NOTHING: stamp-handle.sh and keeper-launch.sh are CALLED UNEDITED
# (2.765 rule 5); every refusal they own (42/43/46, 71..75) stays theirs. It can
# only ever ADD a handle+keeper, never change a verdict: every path returns 0 and
# the runner's rc is still rollup's. Set RAT_KEEPER=0 to arm neither.
arm_own_handle() {
    local hf="${RESULTS}.handle" out hrc
    [ "${RAT_KEEPER:-1}" = 0 ] && { echo "[run-all] handle: RAT_KEEPER=0, arming neither handle nor keeper"; return 0; }
    [ -f scripts/stamp-handle.sh ] && [ -f scripts/keeper-launch.sh ] || return 0
    # An existing handle is left alone: re-stamping is idempotent by design, but a
    # handle already CLOSED by --exit must not be re-opened, and one stamped by an
    # agent already names the writer we would name.
    # A handle belonging to a walk that has ENDED would block all three halves of
    # this one (no stamp, no keeper -- 74, and the 2.858 trap leaves an existing
    # EXIT row alone). 2.907 MEASURED the hole in the old condition: it asked for
    # "EXIT row present AND 0 rows", which covers a walk that CLOSED OUT and
    # misses the case that actually happens -- a walk that DIES without closing
    # out leaves an OPEN handle naming a DEAD winpid, which then disarms every
    # later walk on that file forever (launch #1: keeper WRITER-GONE 65, holders
    # ARM-REFUSED 33, on winpid=31936 dead since 04:46:30Z). 31936 is HISTORY, not
    # a fixture: no handle carrying it survives in the tree. The corpse still ON
    # DISK is the same dead walk 2948's other one --
    # .rat-results.2948.tsv.handle.CONTAMINATED, winpid=30188 -- and that is what
    # scripts/openhandle-integration-2949.sh runs the REAL runner over, 5/5. The widened rule is
    # `0 rows AND (EXIT row OR winpid dead)` and it lives in ONE place --
    # scripts/handle-stale-check.sh, 7/7 arms reached offline by
    # scripts/stalecheck-selftest-2908.sh, including a REAL measured corpse and
    # UNKNOWN-is-not-DEAD. Only rc=0 moves, and it MOVES, never deletes.
    if [ -f "$hf" ] && [ -f scripts/handle-stale-check.sh ]; then
        out="$(bash scripts/handle-stale-check.sh "$RESULTS" 2>&1)"; hrc=$?
        if [ "$hrc" = 0 ]; then
            mv -f "$hf" "${hf}.prev"
            echo "[run-all] handle: $(printf '%s\n' "$out" | tail -1); moved to ${hf}.prev"
        else
            echo "[run-all] handle: $(printf '%s\n' "$out" | tail -1)"
        fi
    fi
    if [ -f "$hf" ]; then
        echo "[run-all] handle: ${hf} already exists; not re-stamping"
    else
        # STAMP_PID=$$ is not a typed pid: stamp-handle.sh only lets it SELECT from
        # the writers ps itself found, and refuses 47 if ps does not list it. It is
        # required here because a self-stamp is always multi-candidate (43).
        out="$(STAMP_PID=$$ bash scripts/stamp-handle.sh "$RESULTS" 2>&1)"
        echo "[run-all] handle: $(printf '%s\n' "$out" | tail -1)"
    fi
    # MEASURED, not predicted: on a SELECTION run the results file is not created
    # here (only `--only`-less walks truncate it above), so the keeper's very first
    # `--rows` call refused 65 "does not exist" and the launch came back 75. An
    # empty file is the truth at this instant -- the walk has written no row yet --
    # and it is the same file the first suite is about to append to.
    [ -f "$RESULTS" ] || : > "$RESULTS"
    out="$(bash scripts/keeper-launch.sh "$RESULTS" "${RAT_KEEPER_INTERVAL:-20}" "${RAT_KEEPER_MAX:-7200}" 2>&1)"
    # tail, not head: MEASURED -- keeper-launch.sh prints an advisory line first
    # when it replaces a stale pidfile, so `head -1` hid the KEEPER-LAUNCHED
    # verdict behind it. Every verdict it can reach is its LAST line.
    echo "[run-all] keeper: $(printf '%s\n' "$out" | tail -1)"
    # ---- the two exit-code holders, armed HERE and not by a launcher (2.891) ----
    # WHY HERE. The handle and the keeper stopped being hand-typed at 2.792; the
    # holders were the last step a launcher still typed by hand, and 2.791's
    # finding is what that costs: two holders cross-AGREEING about a pid that was
    # not the one appending rows. arm-holders.sh already takes both its paths and
    # its pid from ${RESULTS}.handle, so the only remaining way to arm the wrong
    # thing is to not arm at all -- which is what a launcher that forgets does.
    #
    # ARMING AT ROW 0 IS MEASURED, NOT ASSUMED. 2.876 got refusal 36 here and it
    # proved nothing: .rat-verdict-2765.ps1's identity guard (`CommandLine
    # -notmatch run-all-tests.sh`) fires long before any row is counted, so a
    # `sleep` stand-in can never reach the question. armzero-probe-2877.sh re-asked
    # it with a stand-in genuinely NAMED run-all-tests.sh: rc=0, both holders
    # ATTACHED reading `rows=0/684`, and both stamped the real death. Row 0 is a
    # legal arming point; only the stamped CODE was odd (1792 = 7<<8, the MSYS
    # wait status), which is exactly what the 2.886 close-out ruler exists to name.
    #
    # RAT_EXPECT IS PASSED SO THIS DOES NOT RE-ENTER ITSELF. Unset, arm-holders.sh
    # shells out to `run-all-tests.sh --list` (arm-holders.sh:159) -- from inside
    # run-all-tests.sh. That terminates (list mode exits at the MODE=list block,
    # far above this function) but it is a whole slow registry audit under a live
    # walk, and the count it would go and re-derive is already in scope right here.
    #
    # IT CANNOT CHANGE THE VERDICT. rc is captured into a local and only PRINTED;
    # every refusal in the 30-37 band is displayed and swallowed; the function
    # still returns 0 on every path, exactly as the keeper half above does.
    if [ "${RAT_HOLDERS:-1}" != 0 ] && [ -f scripts/arm-holders.sh ]; then
        out="$(RAT_EXPECT="${#SUITES[@]}" bash scripts/arm-holders.sh "$RESULTS" 2>&1)"; hrc=$?
        echo "[run-all] holders: $(printf '%s\n' "$out" | tail -1) (rc=${hrc})"
    else
        echo "[run-all] holders: RAT_HOLDERS=0 or scripts/arm-holders.sh missing; arming neither"
    fi
    return 0
}
arm_own_handle

# Build once up front so each suite's own `cargo build` is a fast no-op.
echo "[run-all] building the kernel once..."
cargo build -p mansa-kernel || { echo "[run-all] BUILD FAILED"; exit 1; }

# A leaked guest is not cosmetic: the next suite binds the same serial and QMP ports,
# so ONE stray QEMU fails the suite AFTER it for a reason that has nothing to do with
# what that suite tests — the exact harness-looks-like-product confusion rat-triage.sh
# exists to separate.
#
# Both halves of the old cleanup could be doing nothing without saying so. `pkill` is
# not installed on the Windows dev box at all (git-bash ships no procps: `pkill` exits
# 127), and `taskkill /IM` named only qemu-system-aarch64.exe while QEMU for Windows
# installs a SECOND binary beside it — qemu-system-aarch64w.exe, the no-console build —
# which that line could never touch. Every stream was sent to /dev/null behind an
# unconditional `return 0`, so killing nothing looked exactly like killing everything.
# Phase 2.886 wrote the replacement and DELIBERATELY did not wire it in, because the
# //IM form was the safety net until the replacement had been measured on a real
# control guest. It has been (reap-probe-2886.sh, 10/0, two live emulators: the MARKED
# guest died, the UNMARKED control lived). So //IM is now GONE from this file, and with
# it the `pkill -f "$QEMU"` fallback, which was the same too-wide match by another name
# (it is not installed here anyway - git-bash exits 127).
#
# What this buys: two walks, or a walk and a hand-run guest, can share this box without
# eating each other. What it COSTS, stated plainly: a stray that was NOT launched
# through the 2.885 shim carries no marker, so this reaper leaves it alone where the
# //IM sweep would have taken it. That is the trade - a reaper that cannot see every
# stray, in exchange for one that cannot kill a guest it does not own.
# 2.873: the MINTED-PID pass. Consumes $QEMU_PIDS_DIR (2.872 gap (b)) and kills by the
# pid the shim wrote, so the per-row cleanup no longer pays a machine-wide command-line
# query it almost always answers "nothing". Empty dir = 126 ms measured, vs 44.6 s for
# the marker reaper's no-op. Named trade: this pass can only see guests THIS walk's shim
# minted, so a marked stray whose shim died before minting is invisible to it - which is
# why the marker reaper is kept for the FINAL sweep instead of being deleted.
kill_qemu_pids() {
    local reaper=scripts/qemu-reap-pids-2873.sh out rc
    [ -f "$reaper" ] && [ -n "${QEMU_PIDS_DIR:-}" ] || return 0
    out="$(bash "$reaper" "$QEMU_PIDS_DIR" 2>&1)"; rc=$?
    if [ "$rc" != 0 ] || printf '%s\n' "$out" | grep -qE '^REAPPIDS (killed|REFUSED|SURVIVOR)'; then
        printf '%s\n' "$out" | sed 's/^/[run-all]   /'
    fi
    return 0
}

kill_qemu() {
    local reaper=scripts/qemu-reap-2886.sh out rc
    [ -f "$reaper" ] || { echo "[run-all] kill_qemu: missing $reaper; killing nothing"; return 0; }
    out="$(bash "$reaper" "afrish-walk-${AFRISH_WALK_ID}" 2>&1)"; rc=$?
    case "$rc" in
        0) printf '%s\n' "$out" | grep -q '^REAP killed' &&
               echo "[run-all] cleaned up a stray QEMU before continuing (by pid, marker afrish-walk-${AFRISH_WALK_ID})" ;;
        *) echo "[run-all] kill_qemu: reaper rc=$rc, nothing is guaranteed dead:"
           printf '%s\n' "$out" | sed 's/^/[run-all]   /' ;;
    esac
    return 0
}

# A VERDICT IS A BANNER, NOT A TOKEN - AND THE RULE LIVES IN ONE FILE
# (3.633, completing the repair 3.632 started and 3.616 measured).
#
# The runner used to decide every row of every walk with two UNANCHORED substring
# greps, `grep -q "PASSED"` and `grep -qE "PASSED|FAILED|FAIL"`. A verdict is a
# BANNER a suite prints about itself; those are merely substrings, and a
# QEMU-booting suite's log carries a whole cargo build, so they appear for reasons
# that have nothing to do with any verdict. On the CLOSED, all-green walk 3600 the
# old FAIL family matched two logs on non-verdict text - rat-pi.log's rustc
# `constant `S_FAILED` is never used` and rat-rxsplit.log's `0 FAIL` self-test
# tally - where a FAIL-family hit is by construction a false positive. Latent at
# rc=0, but had either suite exited non-zero for any reason the runner would have
# called it FAIL ("ran, judged itself, lost") when the truth is DIED ("died before
# reaching its own verdict"). That is 2.766's fragment-matching hazard living in
# the instrument that grades every walk.
#
# The rule itself - the anchors, both branches, and the rc ladder - is
# scripts/lib/verdict.sh, SOURCED here and by scripts/verdict-selftest-3632.sh.
# 3.184: a second copy is a second thing to be wrong, and this exact line proved
# it. 3.632 left the library correct but ALSO left an inline paraphrase here, and
# that paraphrase carried literal 0x08 BACKSPACE bytes where regex word-boundary
# escapes were meant, so it matched 0 of 689 walk-3600 logs and would have graded
# an entire walk NO-VERDICT. Nothing below re-states a regex. One definition or
# none - and verdict-selftest-3632.sh arm "runner holds no second copy" greps this
# file to keep it that way.
VERDICT_LIB="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/verdict.sh"
[ -f "$VERDICT_LIB" ] || {
    echo "[run-all] FATAL: missing $VERDICT_LIB - the verdict grader is not optional"; exit 2; }
# shellcheck source=lib/verdict.sh
. "$VERDICT_LIB"
command -v classify_verdict >/dev/null 2>&1 || {
    echo "[run-all] FATAL: $VERDICT_LIB defined no classify_verdict"; exit 2; }

echo "[run-all] running ${#TORUN[@]} of ${#SUITES[@]} registered suites, ${TIMEOUT}s per-suite timeout"
i=0
for s in "${TORUN[@]}"; do
    i=$((i + 1))
    kill_qemu_pids
    sleep 1
    script="scripts/${s}-test.sh"
    log="${LOGDIR}/rat-${s}.log"
    t0=$SECONDS
    printf '[run-all] (%d/%d) --- %s ---\n' "$i" "${#TORUN[@]}" "$(basename "$script")"
    if [ ! -f "$script" ]; then
        st=MISSING; rc=-1                       # registered, but there is nothing to run
    else
        cap=$(suite_timeout "$s")
        [ "$cap" != "$TIMEOUT" ] &&
            printf '[run-all] per-suite cap: %s gets %ss (global %ss)\n' "$s" "$cap" "$TIMEOUT"
        timeout -k 5 "$cap" bash "$script" > "$log" 2>&1
        rc=$?
        st="$(classify_verdict "$rc" "$log")"
    fi
    sec=$((SECONDS - t0))
    # Dated at completion, i.e. AFTER the suite's last write to its log and capture,
    # so evidence for this row can never legitimately be newer than the row itself.
    ts="$(date +%s)"
    printf '%s\t%s\t%s\t%s\t%s\n' "$s" "$st" "$sec" "$rc" "$ts" >> "$RESULTS"
    RECORDED[$s]="${st}"$'\t'"${sec}"$'\t'"${rc}"$'\t'"${ts}"
    if [ "$st" != PASS ]; then
        echo "[run-all] ${s} ${st} (rc=${rc}, ${sec}s); tail of its log:"
        [ -f "$log" ] && tail -5 "$log" | sed 's/^/    /'
    fi
done
# The walk's LAST cleanup pays for both: the pid pass consumes what the shim minted,
# then the marker reaper takes anything carrying this walk's -name that never got a
# pid file. Once per walk, not once per row.
kill_qemu_pids
kill_qemu

rollup

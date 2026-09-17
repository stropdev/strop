(* automatically generated -- do not edit manually *)
theory GraphTheorem imports Constant Zenon begin
ML_command {* writeln ("*** TLAPS PARSED\n"); *}
consts
  "isReal" :: c
  "isa_slas_a" :: "[c,c] => c"
  "isa_bksl_diva" :: "[c,c] => c"
  "isa_perc_a" :: "[c,c] => c"
  "isa_peri_peri_a" :: "[c,c] => c"
  "isInfinity" :: c
  "isa_lbrk_rbrk_a" :: "[c] => c"
  "isa_less_more_a" :: "[c] => c"

lemma ob'67:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
(* usable definition CONSTANT_Edges_ suppressed *)
(* usable definition CONSTANT_NonLoopEdges_ suppressed *)
(* usable definition CONSTANT_SimpleGraphs_ suppressed *)
(* usable definition CONSTANT_Degree_ suppressed *)
fixes a_CONSTANTunde_Nodesunde_a
assumes v'155: "((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a))))"
assumes v'156: "((greater (((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Nodesunde_a)))), ((Succ[0])))))"
fixes a_CONSTANTunde_Gunde_a
assumes a_CONSTANTunde_Gunde_a_in : "(a_CONSTANTunde_Gunde_a \<in> ((a_CONSTANTunde_SimpleGraphsunde_a ((a_CONSTANTunde_Nodesunde_a)))))"
fixes a_CONSTANTunde_eunde_a
assumes a_CONSTANTunde_eunde_a_in : "(a_CONSTANTunde_eunde_a \<in> (a_CONSTANTunde_Gunde_a))"
fixes a_CONSTANTunde_nunde_a
assumes a_CONSTANTunde_nunde_a_in : "(a_CONSTANTunde_nunde_a \<in> (a_CONSTANTunde_eunde_a))"
assumes v'179: "((a_CONSTANTunde_IsFiniteSetunde_a ((subsetOf((a_CONSTANTunde_Gunde_a), %a_CONSTANTunde_eeunde_a. (((a_CONSTANTunde_nunde_a) \<in> (a_CONSTANTunde_eeunde_a))))))))"
assumes v'180: "(((a_CONSTANTunde_IsFiniteSetunde_a (({})))) & (\<forall>a_CONSTANTunde_Sunde_a : ((((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Sunde_a)))) \<Rightarrow> ((((((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Sunde_a)))) = ((0)))) \<Leftrightarrow> (((a_CONSTANTunde_Sunde_a) = ({})))))))))"
assumes v'181: "((((a_CONSTANTunde_Cardinalityunde_a (({})))) = ((0))))"
shows "((((a_CONSTANTunde_Cardinalityunde_a ((subsetOf((a_CONSTANTunde_Gunde_a), %a_CONSTANTunde_eeunde_a. (((a_CONSTANTunde_nunde_a) \<in> (a_CONSTANTunde_eeunde_a)))))))) \<noteq> ((0))))"(is "PROP ?ob'67")
proof -
ML_command {* writeln "*** TLAPS ENTER 67"; *}
show "PROP ?ob'67"
using assms by auto
ML_command {* writeln "*** TLAPS EXIT 67"; *} qed
lemma ob'63:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
(* usable definition CONSTANT_Edges_ suppressed *)
(* usable definition CONSTANT_Degree_ suppressed *)
fixes a_CONSTANTunde_Nodesunde_a
assumes v'153: "((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a))))"
assumes v'154: "((greater (((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Nodesunde_a)))), ((Succ[0])))))"
fixes a_CONSTANTunde_Gunde_a
assumes a_CONSTANTunde_Gunde_a_in : "(a_CONSTANTunde_Gunde_a \<in> ((SUBSET (subsetOf(((a_CONSTANTunde_Edgesunde_a ((a_CONSTANTunde_Nodesunde_a)))), %a_CONSTANTunde_eunde_a. ((((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_eunde_a)))) = ((Succ[Succ[0]])))))))))"
fixes a_CONSTANTunde_eunde_a
assumes a_CONSTANTunde_eunde_a_in : "(a_CONSTANTunde_eunde_a \<in> (a_CONSTANTunde_Gunde_a))"
fixes a_CONSTANTunde_nunde_a
assumes a_CONSTANTunde_nunde_a_in : "(a_CONSTANTunde_nunde_a \<in> (a_CONSTANTunde_eunde_a))"
assumes v'176: "((\<And> a_CONSTANTunde_Nodesunde_a_1 :: c. (((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a_1)))) \<Longrightarrow> ((a_CONSTANTunde_IsFiniteSetunde_a (((a_CONSTANTunde_Edgesunde_a ((a_CONSTANTunde_Nodesunde_a_1))))))))))"
assumes v'177: "((\<And> a_CONSTANTunde_Sunde_a :: c. (((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Sunde_a)))) \<Longrightarrow> (\<And> a_CONSTANTunde_Tunde_a :: c. a_CONSTANTunde_Tunde_a \<in> ((SUBSET (a_CONSTANTunde_Sunde_a))) \<Longrightarrow> (((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Tunde_a)))) & ((leq (((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Tunde_a)))), ((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Sunde_a))))))) & ((((((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Sunde_a)))) = ((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Tunde_a)))))) \<Rightarrow> (((a_CONSTANTunde_Sunde_a) = (a_CONSTANTunde_Tunde_a))))))))))"
shows "((a_CONSTANTunde_IsFiniteSetunde_a ((subsetOf((a_CONSTANTunde_Gunde_a), %a_CONSTANTunde_eeunde_a. (((a_CONSTANTunde_nunde_a) \<in> (a_CONSTANTunde_eeunde_a))))))))"(is "PROP ?ob'63")
proof -
ML_command {* writeln "*** TLAPS ENTER 63"; *}
show "PROP ?ob'63"

(* BEGIN ZENON INPUT
;; file=.tlacache/GraphTheorem.tlaps/tlapm_3a8c4e.znn; PATH='/tmp/tlaps/inst/bin:/home/tarek/.kimi-code/bin:/home/tarek/.local/share/swiftly/bin:/home/tarek/.rbenv/shims:/run/user/1000/fnm_multishells/98560_1789588384174/bin:/home/tarek/.local/share/fnm:/home/tarek/.rbenv/shims:/home/tarek/.rbenv/bin:/home/tarek/.local/bin:/home/tarek/.cargo/bin:/run/user/1000/fnm_multishells/581_1789534734414/bin:/home/tarek/.local/share/fnm:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games:/usr/local/games:/usr/lib/wsl/lib:/mnt/c/Program Files/coreutils/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.2/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.2/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.1/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.1/bin:/mnt/c/WINDOWS/system32:/mnt/c/WINDOWS:/mnt/c/WINDOWS/System32/Wbem:/mnt/c/WINDOWS/System32/WindowsPowerShell/v1.0/:/mnt/c/WINDOWS/System32/OpenSSH/:/mnt/c/Program Files/Microsoft SQL Server/170/Tools/Binn/:/mnt/c/Program Files/Microsoft SQL Server/Client SDK/ODBC/170/Tools/Binn/:/mnt/c/Program Files/NVIDIA Corporation/Nsight Compute 2026.2.0/:/mnt/c/Program Files (x86)/Windows Kits/10/Windows Performance Toolkit/:/mnt/c/Program Files/NVIDIA Corporation/NVIDIA App/NvDLISR:/mnt/c/Program Files/dotnet/:/mnt/c/Program Files/CMake/bin:/mnt/c/Program Files/Docker/Docker/resources/bin:/mnt/c/Program Files/Git/cmd:/mnt/c/Program Files/GitHub CLI/:/mnt/c/Users/tarek/.kimi-code/bin:/mnt/c/Users/tarek/AppData/Local/Microsoft/WindowsApps:/mnt/c/Users/tarek/AppData/Local/Programs/Microsoft VS Code/bin:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/Programs/codecrafters:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/SST.opencode_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/AppData/Local/Programs/Ollama:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/Schniz.fnm_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/sinelaw.fresh-editor_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Links:/mnt/c/Users/tarek/AppData/Local/Programs/Zed/bin:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/PowerToys/DSCModules/:/snap/bin:/tmp/tlaps/inst/bin:/tmp/tlaps/inst/lib/tlaps/bin'; zenon -p0 -x tla -oisar -max-time 1d "$file" >.tlacache/GraphTheorem.tlaps/tlapm_3a8c4e.znn.out
;; obligation #63
$hyp "v'153" (a_CONSTANTunde_IsFiniteSetunde_a a_CONSTANTunde_Nodesunde_a)
$hyp "v'154" (arith.lt (TLA.fapply TLA.Succ 0)
(a_CONSTANTunde_Cardinalityunde_a a_CONSTANTunde_Nodesunde_a))
$hyp "a_CONSTANTunde_Gunde_a_in" (TLA.in a_CONSTANTunde_Gunde_a (TLA.SUBSET (TLA.subsetOf (a_CONSTANTunde_Edgesunde_a a_CONSTANTunde_Nodesunde_a) ((a_CONSTANTunde_eunde_a) (= (a_CONSTANTunde_Cardinalityunde_a a_CONSTANTunde_eunde_a)
(TLA.fapply TLA.Succ (TLA.fapply TLA.Succ 0)))))))
$hyp "a_CONSTANTunde_eunde_a_in" (TLA.in a_CONSTANTunde_eunde_a a_CONSTANTunde_Gunde_a)
$hyp "a_CONSTANTunde_nunde_a_in" (TLA.in a_CONSTANTunde_nunde_a a_CONSTANTunde_eunde_a)
$hyp "v'176" (A. ((a_CONSTANTunde_Nodesunde_a_1) (=> (a_CONSTANTunde_IsFiniteSetunde_a a_CONSTANTunde_Nodesunde_a_1) (a_CONSTANTunde_IsFiniteSetunde_a (a_CONSTANTunde_Edgesunde_a a_CONSTANTunde_Nodesunde_a_1)))))
$hyp "v'177" (A. ((a_CONSTANTunde_Sunde_a) (=> (a_CONSTANTunde_IsFiniteSetunde_a a_CONSTANTunde_Sunde_a) (TLA.bAll (TLA.SUBSET a_CONSTANTunde_Sunde_a) ((a_CONSTANTunde_Tunde_a) (/\ (a_CONSTANTunde_IsFiniteSetunde_a a_CONSTANTunde_Tunde_a)
(arith.le (a_CONSTANTunde_Cardinalityunde_a a_CONSTANTunde_Tunde_a)
(a_CONSTANTunde_Cardinalityunde_a a_CONSTANTunde_Sunde_a))
(=> (= (a_CONSTANTunde_Cardinalityunde_a a_CONSTANTunde_Sunde_a)
(a_CONSTANTunde_Cardinalityunde_a a_CONSTANTunde_Tunde_a))
(= a_CONSTANTunde_Sunde_a
a_CONSTANTunde_Tunde_a))))))))
$goal (a_CONSTANTunde_IsFiniteSetunde_a (TLA.subsetOf a_CONSTANTunde_Gunde_a ((a_CONSTANTunde_eeunde_a) (TLA.in a_CONSTANTunde_nunde_a
a_CONSTANTunde_eeunde_a))))
END ZENON  INPUT *)
(* PROOF-FOUND *)
(* BEGIN-PROOF *)
proof (rule zenon_nnpp)
 have z_Hc:"(a_CONSTANTunde_Gunde_a \\in SUBSET(subsetOf(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_eunde_a. (a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_eunde_a)=2)))))" (is "?z_hc")
 using a_CONSTANTunde_Gunde_a_in by blast
 have z_Ha:"a_CONSTANTunde_IsFiniteSetunde_a(a_CONSTANTunde_Nodesunde_a)" (is "?z_ha")
 using v'153 by blast
 have z_Hf:"(\\A a_CONSTANTunde_Nodesunde_a_1:(a_CONSTANTunde_IsFiniteSetunde_a(a_CONSTANTunde_Nodesunde_a_1)=>a_CONSTANTunde_IsFiniteSetunde_a(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a_1))))" (is "\\A x : ?z_hx(x)")
 using v'176 by blast
 have z_Hg:"(\\A a_CONSTANTunde_Sunde_a:(a_CONSTANTunde_IsFiniteSetunde_a(a_CONSTANTunde_Sunde_a)=>bAll(SUBSET(a_CONSTANTunde_Sunde_a), (\<lambda>a_CONSTANTunde_Tunde_a. (a_CONSTANTunde_IsFiniteSetunde_a(a_CONSTANTunde_Tunde_a)&((a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_Tunde_a) <= a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_Sunde_a))&((a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_Sunde_a)=a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_Tunde_a))=>(a_CONSTANTunde_Sunde_a=a_CONSTANTunde_Tunde_a))))))))" (is "\\A x : ?z_hbo(x)")
 using v'177 by blast
 assume z_Hh:"(~a_CONSTANTunde_IsFiniteSetunde_a(subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a)))))" (is "~?z_hbp")
 have z_Hbv: "(a_CONSTANTunde_Gunde_a \\subseteq subsetOf(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_eunde_a. (a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_eunde_a)=2))))" (is "?z_hbv")
 by (rule zenon_in_SUBSET_0 [of "a_CONSTANTunde_Gunde_a" "subsetOf(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_eunde_a. (a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_eunde_a)=2)))", OF z_Hc])
 have z_Hbw_z_Hbv: "bAll(a_CONSTANTunde_Gunde_a, (\<lambda>x. (x \\in subsetOf(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_eunde_a. (a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_eunde_a)=2)))))) == ?z_hbv" (is "?z_hbw == _")
 by (unfold subset_def)
 have z_Hbw: "?z_hbw"
 by (unfold z_Hbw_z_Hbv, fact z_Hbv)
 have z_Hca_z_Hbw: "(\\A x:((x \\in a_CONSTANTunde_Gunde_a)=>(x \\in subsetOf(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_eunde_a. (a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_eunde_a)=2)))))) == ?z_hbw" (is "?z_hca == _")
 by (unfold bAll_def)
 have z_Hca: "?z_hca" (is "\\A x : ?z_hcd(x)")
 by (unfold z_Hca_z_Hbw, fact z_Hbw)
 have z_Hce: "?z_hbo(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a))" (is "?z_hcf=>?z_hcg")
 by (rule zenon_all_0 [of "?z_hbo" "a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)", OF z_Hg])
 show FALSE
 proof (rule zenon_imply [OF z_Hce])
  assume z_Hch:"(~?z_hcf)"
  have z_Hci: "?z_hx(a_CONSTANTunde_Nodesunde_a)"
  by (rule zenon_all_0 [of "?z_hx" "a_CONSTANTunde_Nodesunde_a", OF z_Hf])
  show FALSE
  proof (rule zenon_imply [OF z_Hci])
   assume z_Hcj:"(~?z_ha)"
   show FALSE
   by (rule notE [OF z_Hcj z_Ha])
  next
   assume z_Hcf:"?z_hcf"
   show FALSE
   by (rule notE [OF z_Hch z_Hcf])
  qed
 next
  assume z_Hcg:"?z_hcg"
  have z_Hck_z_Hcg: "(\\A x:((x \\in SUBSET(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)))=>(a_CONSTANTunde_IsFiniteSetunde_a(x)&((a_CONSTANTunde_Cardinalityunde_a(x) <= a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)))&((a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a))=a_CONSTANTunde_Cardinalityunde_a(x))=>(a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)=x)))))) == ?z_hcg" (is "?z_hck == _")
  by (unfold bAll_def)
  have z_Hck: "?z_hck" (is "\\A x : ?z_hcx(x)")
  by (unfold z_Hck_z_Hcg, fact z_Hcg)
  have z_Hcy: "?z_hcx(subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))" (is "?z_hcz=>?z_hda")
  by (rule zenon_all_0 [of "?z_hcx" "subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a)))", OF z_Hck])
  show FALSE
  proof (rule zenon_imply [OF z_Hcy])
   assume z_Hdb:"(~?z_hcz)"
   have z_Hdc: "(~(subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))) \\subseteq a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)))" (is "~?z_hdd")
   by (rule zenon_notin_SUBSET_0 [of "subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a)))" "a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)", OF z_Hdb])
   have z_Hde_z_Hdc: "(~bAll(subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))), (\<lambda>zenon_Vx. (zenon_Vx \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a))))) == (~?z_hdd)" (is "?z_hde == ?z_hdc")
   by (unfold subset_def)
   have z_Hde: "?z_hde" (is "~?z_hdf")
   by (unfold z_Hde_z_Hdc, fact z_Hdc)
   have z_Hdj_z_Hde: "(~(\\A x:((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a))))) == ?z_hde" (is "?z_hdj == _")
   by (unfold bAll_def)
   have z_Hdj: "?z_hdj" (is "~(\\A x : ?z_hdo(x))")
   by (unfold z_Hdj_z_Hde, fact z_Hde)
   have z_Hdp: "(\\E x:(~((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)))))" (is "\\E x : ?z_hdr(x)")
   by (rule zenon_notallex_0 [of "?z_hdo", OF z_Hdj])
   have z_Hds: "?z_hdr((CHOOSE x:(~((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a))))))" (is "~(?z_hdu=>?z_hdv)")
   by (rule zenon_ex_choose_0 [of "?z_hdr", OF z_Hdp])
   have z_Hdu: "?z_hdu"
   by (rule zenon_notimply_0 [OF z_Hds])
   have z_Hdw: "(~?z_hdv)"
   by (rule zenon_notimply_1 [OF z_Hds])
   have z_Hdx: "((CHOOSE x:(~((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a))))) \\in a_CONSTANTunde_Gunde_a)" (is "?z_hdx")
   by (rule zenon_in_subsetof_0 [of "(CHOOSE x:(~((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)))))" "a_CONSTANTunde_Gunde_a" "(\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))", OF z_Hdu])
   have z_Hdy: "?z_hcd((CHOOSE x:(~((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a))))))" (is "_=>?z_hdz")
   by (rule zenon_all_0 [of "?z_hcd" "(CHOOSE x:(~((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)))))", OF z_Hca])
   show FALSE
   proof (rule zenon_imply [OF z_Hdy])
    assume z_Hea:"(~?z_hdx)"
    show FALSE
    by (rule notE [OF z_Hea z_Hdx])
   next
    assume z_Hdz:"?z_hdz"
    have z_Hdv: "?z_hdv"
    by (rule zenon_in_subsetof_0 [of "(CHOOSE x:(~((x \\in subsetOf(a_CONSTANTunde_Gunde_a, (\<lambda>a_CONSTANTunde_eeunde_a. (a_CONSTANTunde_nunde_a \\in a_CONSTANTunde_eeunde_a))))=>(x \\in a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)))))" "a_CONSTANTunde_Edgesunde_a(a_CONSTANTunde_Nodesunde_a)" "(\<lambda>a_CONSTANTunde_eunde_a. (a_CONSTANTunde_Cardinalityunde_a(a_CONSTANTunde_eunde_a)=2))", OF z_Hdz])
    show FALSE
    by (rule notE [OF z_Hdw z_Hdv])
   qed
  next
   assume z_Hda:"?z_hda" (is "_&?z_heb")
   have z_Hbp: "?z_hbp"
   by (rule zenon_and_0 [OF z_Hda])
   show FALSE
   by (rule notE [OF z_Hh z_Hbp])
  qed
 qed
qed
(* END-PROOF *)
ML_command {* writeln "*** TLAPS EXIT 63"; *} qed
lemma ob'12:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
(* usable definition CONSTANT_Edges_ suppressed *)
(* usable definition CONSTANT_SimpleGraphs_ suppressed *)
(* usable definition CONSTANT_Degree_ suppressed *)
fixes a_CONSTANTunde_Nodesunde_a
assumes v'153: "((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a))))"
fixes a_CONSTANTunde_eunde_a
assumes a_CONSTANTunde_eunde_a_in : "(a_CONSTANTunde_eunde_a \<in> (subsetOf(((a_CONSTANTunde_Edgesunde_a ((a_CONSTANTunde_Nodesunde_a)))), %a_CONSTANTunde_eunde_a. ((((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_eunde_a)))) = ((Succ[Succ[0]])))))))"
fixes a_CONSTANTunde_munde_a
assumes a_CONSTANTunde_munde_a_in : "(a_CONSTANTunde_munde_a \<in> (a_CONSTANTunde_Nodesunde_a))"
fixes a_CONSTANTunde_nunde_a
assumes a_CONSTANTunde_nunde_a_in : "(a_CONSTANTunde_nunde_a \<in> (a_CONSTANTunde_Nodesunde_a))"
assumes v'158: "(((a_CONSTANTunde_eunde_a) = ({(a_CONSTANTunde_munde_a), (a_CONSTANTunde_nunde_a)})))"
assumes v'161: "(((a_CONSTANTunde_munde_a) = (a_CONSTANTunde_nunde_a)))"
assumes v'164: "((\<forall>a_CONSTANTunde_xunde_a : ((a_CONSTANTunde_IsFiniteSetunde_a (({(a_CONSTANTunde_xunde_a)}))))) & (\<forall>a_CONSTANTunde_Sunde_a : ((((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Sunde_a)))) \<Rightarrow> ((((((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Sunde_a)))) = ((Succ[0])))) \<Leftrightarrow> (\<exists>a_CONSTANTunde_xunde_a : (((a_CONSTANTunde_Sunde_a) = ({(a_CONSTANTunde_xunde_a)}))))))))))"
assumes v'165: "(\<forall>a_CONSTANTunde_xunde_a : ((((a_CONSTANTunde_Cardinalityunde_a (({(a_CONSTANTunde_xunde_a)})))) = ((Succ[0])))))"
shows "(FALSE)"(is "PROP ?ob'12")
proof -
ML_command {* writeln "*** TLAPS ENTER 12"; *}
show "PROP ?ob'12"
using assms by auto
ML_command {* writeln "*** TLAPS EXIT 12"; *} qed
lemma ob'9:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
fixes a_CONSTANTunde_Nodesunde_a
assumes v'149: "((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a))))"
assumes v'153: "((\<And> a_CONSTANTunde_Sunde_a :: c. (\<And> a_CONSTANTunde_Tunde_a :: c. (\<And> a_CONSTANTunde_Funde_a :: c. a_CONSTANTunde_Funde_a \<in> ([(a_CONSTANTunde_Sunde_a) \<rightarrow> (a_CONSTANTunde_Tunde_a)]) \<Longrightarrow> ((\<forall> a_CONSTANTunde_tunde_a \<in> (a_CONSTANTunde_Tunde_a) : (\<exists> a_CONSTANTunde_sunde_a \<in> (a_CONSTANTunde_Sunde_a) : (((fapply ((a_CONSTANTunde_Funde_a), (a_CONSTANTunde_sunde_a))) = (a_CONSTANTunde_tunde_a))))) \<Longrightarrow> (((a_CONSTANTunde_Funde_a) \<in> ((a_CONSTANTunde_Surjectionunde_a ((a_CONSTANTunde_Sunde_a), (a_CONSTANTunde_Tunde_a)))))))))))"
shows "((([ a_CONSTANTunde_munde_a \<in> (((a_CONSTANTunde_Nodesunde_a) \<times> (a_CONSTANTunde_Nodesunde_a)))  \<mapsto> ({(fapply ((a_CONSTANTunde_munde_a), ((Succ[0])))), (fapply ((a_CONSTANTunde_munde_a), ((Succ[Succ[0]]))))})]) \<in> ((a_CONSTANTunde_Surjectionunde_a ((((a_CONSTANTunde_Nodesunde_a) \<times> (a_CONSTANTunde_Nodesunde_a))), (setOfAll((((a_CONSTANTunde_Nodesunde_a) \<times> (a_CONSTANTunde_Nodesunde_a))), %a_CONSTANTunde_munde_a. ({(fapply ((a_CONSTANTunde_munde_a), ((Succ[0])))), (fapply ((a_CONSTANTunde_munde_a), ((Succ[Succ[0]]))))}))))))))"(is "PROP ?ob'9")
proof -
ML_command {* writeln "*** TLAPS ENTER 9"; *}
show "PROP ?ob'9"

(* BEGIN ZENON INPUT
;; file=.tlacache/GraphTheorem.tlaps/tlapm_222ecf.znn; PATH='/tmp/tlaps/inst/bin:/home/tarek/.kimi-code/bin:/home/tarek/.local/share/swiftly/bin:/home/tarek/.rbenv/shims:/run/user/1000/fnm_multishells/98560_1789588384174/bin:/home/tarek/.local/share/fnm:/home/tarek/.rbenv/shims:/home/tarek/.rbenv/bin:/home/tarek/.local/bin:/home/tarek/.cargo/bin:/run/user/1000/fnm_multishells/581_1789534734414/bin:/home/tarek/.local/share/fnm:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games:/usr/local/games:/usr/lib/wsl/lib:/mnt/c/Program Files/coreutils/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.2/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.2/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.1/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.1/bin:/mnt/c/WINDOWS/system32:/mnt/c/WINDOWS:/mnt/c/WINDOWS/System32/Wbem:/mnt/c/WINDOWS/System32/WindowsPowerShell/v1.0/:/mnt/c/WINDOWS/System32/OpenSSH/:/mnt/c/Program Files/Microsoft SQL Server/170/Tools/Binn/:/mnt/c/Program Files/Microsoft SQL Server/Client SDK/ODBC/170/Tools/Binn/:/mnt/c/Program Files/NVIDIA Corporation/Nsight Compute 2026.2.0/:/mnt/c/Program Files (x86)/Windows Kits/10/Windows Performance Toolkit/:/mnt/c/Program Files/NVIDIA Corporation/NVIDIA App/NvDLISR:/mnt/c/Program Files/dotnet/:/mnt/c/Program Files/CMake/bin:/mnt/c/Program Files/Docker/Docker/resources/bin:/mnt/c/Program Files/Git/cmd:/mnt/c/Program Files/GitHub CLI/:/mnt/c/Users/tarek/.kimi-code/bin:/mnt/c/Users/tarek/AppData/Local/Microsoft/WindowsApps:/mnt/c/Users/tarek/AppData/Local/Programs/Microsoft VS Code/bin:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/Programs/codecrafters:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/SST.opencode_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/AppData/Local/Programs/Ollama:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/Schniz.fnm_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/sinelaw.fresh-editor_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Links:/mnt/c/Users/tarek/AppData/Local/Programs/Zed/bin:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/PowerToys/DSCModules/:/snap/bin:/tmp/tlaps/inst/bin:/tmp/tlaps/inst/lib/tlaps/bin'; zenon -p0 -x tla -oisar -max-time 1d "$file" >.tlacache/GraphTheorem.tlaps/tlapm_222ecf.znn.out
;; obligation #9
$hyp "v'149" (a_CONSTANTunde_IsFiniteSetunde_a a_CONSTANTunde_Nodesunde_a)
$hyp "v'153" (A. ((a_CONSTANTunde_Sunde_a) (A. ((a_CONSTANTunde_Tunde_a) (TLA.bAll (TLA.FuncSet a_CONSTANTunde_Sunde_a a_CONSTANTunde_Tunde_a) ((a_CONSTANTunde_Funde_a) (=> (TLA.bAll a_CONSTANTunde_Tunde_a ((a_CONSTANTunde_tunde_a) (TLA.bEx a_CONSTANTunde_Sunde_a ((a_CONSTANTunde_sunde_a) (= (TLA.fapply a_CONSTANTunde_Funde_a a_CONSTANTunde_sunde_a)
a_CONSTANTunde_tunde_a))))) (TLA.in a_CONSTANTunde_Funde_a
(a_CONSTANTunde_Surjectionunde_a a_CONSTANTunde_Sunde_a
a_CONSTANTunde_Tunde_a)))))))))
$goal (TLA.in (TLA.Fcn (TLA.prod a_CONSTANTunde_Nodesunde_a a_CONSTANTunde_Nodesunde_a) ((a_CONSTANTunde_munde_a) (TLA.set (TLA.fapply a_CONSTANTunde_munde_a (TLA.fapply TLA.Succ 0)) (TLA.fapply a_CONSTANTunde_munde_a (TLA.fapply TLA.Succ (TLA.fapply TLA.Succ 0))))))
(a_CONSTANTunde_Surjectionunde_a (TLA.prod a_CONSTANTunde_Nodesunde_a a_CONSTANTunde_Nodesunde_a)
(TLA.setOfAll (TLA.prod a_CONSTANTunde_Nodesunde_a a_CONSTANTunde_Nodesunde_a) ((a_CONSTANTunde_munde_a) (TLA.set (TLA.fapply a_CONSTANTunde_munde_a (TLA.fapply TLA.Succ 0)) (TLA.fapply a_CONSTANTunde_munde_a (TLA.fapply TLA.Succ (TLA.fapply TLA.Succ 0))))))))
END ZENON  INPUT *)
(* PROOF-FOUND *)
(* BEGIN-PROOF *)
proof (rule zenon_nnpp)
 have z_Hb:"(\\A a_CONSTANTunde_Sunde_a:(\\A a_CONSTANTunde_Tunde_a:bAll(FuncSet(a_CONSTANTunde_Sunde_a, a_CONSTANTunde_Tunde_a), (\<lambda>a_CONSTANTunde_Funde_a. (bAll(a_CONSTANTunde_Tunde_a, (\<lambda>a_CONSTANTunde_tunde_a. bEx(a_CONSTANTunde_Sunde_a, (\<lambda>a_CONSTANTunde_sunde_a. ((a_CONSTANTunde_Funde_a[a_CONSTANTunde_sunde_a])=a_CONSTANTunde_tunde_a)))))=>(a_CONSTANTunde_Funde_a \\in a_CONSTANTunde_Surjectionunde_a(a_CONSTANTunde_Sunde_a, a_CONSTANTunde_Tunde_a)))))))" (is "\\A x : ?z_hv(x)")
 using v'153 by blast
 have zenon_L1_: "(~bAll(setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})), (\<lambda>a_CONSTANTunde_tunde_a. bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=a_CONSTANTunde_tunde_a)))))) ==> FALSE" (is "?z_hw ==> FALSE")
 proof -
  assume z_Hw:"?z_hw" (is "~?z_hx")
  have z_Hbq_z_Hw: "(~bAll(setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})), (\<lambda>a_CONSTANTunde_tunde_a. bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=a_CONSTANTunde_tunde_a)))))) == ?z_hw" (is "?z_hbq == _")
  by (unfold prod_def)
  have z_Hbq: "?z_hbq" (is "~?z_hbr")
  by (unfold z_Hbq_z_Hw, fact z_Hw)
  have z_Hbt_z_Hbq: "(~(\\A x:((x \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))=>bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=x)))))) == ?z_hbq" (is "?z_hbt == _")
  by (unfold bAll_def)
  have z_Hbt: "?z_hbt" (is "~(\\A x : ?z_hcb(x))")
  by (unfold z_Hbt_z_Hbq, fact z_Hbq)
  have z_Hcc: "(\\E x:(~((x \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))=>bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=x))))))" (is "\\E x : ?z_hce(x)")
  by (rule zenon_notallex_0 [of "?z_hcb", OF z_Hbt])
  have z_Hcf: "?z_hce((CHOOSE x:(~((x \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))=>bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=x)))))))" (is "~(?z_hch=>?z_hci)")
  by (rule zenon_ex_choose_0 [of "?z_hce", OF z_Hcc])
  have z_Hch: "?z_hch"
  by (rule zenon_notimply_0 [OF z_Hcf])
  have z_Hcj: "(~?z_hci)"
  by (rule zenon_notimply_1 [OF z_Hcf])
  have z_Hck_z_Hcj: "(~(\\E x:((x \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[x])=(CHOOSE x:(~((x \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))=>bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=x)))))))))) == (~?z_hci)" (is "?z_hck == ?z_hcj")
  by (unfold bEx_def)
  have z_Hck: "?z_hck" (is "~(\\E x : ?z_hcq(x))")
  by (unfold z_Hck_z_Hcj, fact z_Hcj)
  have z_Hcr: "(\\E zenon_Vvd:((zenon_Vvd \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&((CHOOSE x:(~((x \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))=>bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=x))))))={(zenon_Vvd[1]), (zenon_Vvd[2])})))" (is "\\E x : ?z_hcz(x)")
  by (rule zenon_in_setofall_0 [of "(CHOOSE x:(~((x \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))=>bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=x))))))" "Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>)" "(\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})", OF z_Hch])
  have z_Hda: "?z_hcz((CHOOSE zenon_Vvd:((zenon_Vvd \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&((CHOOSE x:(~((x \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))=>bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[a_CONSTANTunde_sunde_a])=x))))))={(zenon_Vvd[1]), (zenon_Vvd[2])}))))" (is "?z_hdc&?z_hdd")
  by (rule zenon_ex_choose_0 [of "?z_hcz", OF z_Hcr])
  have z_Hdc: "?z_hdc"
  by (rule zenon_and_0 [OF z_Hda])
  have z_Hdd: "?z_hdd" (is "?z_hcg=?z_hde")
  by (rule zenon_and_1 [OF z_Hda])
  have z_Hdf: "~?z_hcq((CHOOSE zenon_Vvd:((zenon_Vvd \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&(?z_hcg={(zenon_Vvd[1]), (zenon_Vvd[2])}))))" (is "~(_&?z_hdg)")
  by (rule zenon_notex_0 [of "?z_hcq" "(CHOOSE zenon_Vvd:((zenon_Vvd \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&(?z_hcg={(zenon_Vvd[1]), (zenon_Vvd[2])})))", OF z_Hck])
  show FALSE
  proof (rule zenon_notand [OF z_Hdf])
   assume z_Hdh:"(~?z_hdc)"
   show FALSE
   by (rule notE [OF z_Hdh z_Hdc])
  next
   assume z_Hdi:"((Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[(CHOOSE zenon_Vvd:((zenon_Vvd \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&(?z_hcg={(zenon_Vvd[1]), (zenon_Vvd[2])})))])~=?z_hcg)" (is "?z_hdj~=_")
   show FALSE
   proof (rule zenon_fapplyfcn [of "(\<lambda>zenon_Vyd. (zenon_Vyd~=?z_hcg))" "prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a)" "(\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})" "(CHOOSE zenon_Vvd:((zenon_Vvd \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&(?z_hcg={(zenon_Vvd[1]), (zenon_Vvd[2])})))", OF z_Hdi])
    assume z_Hdn:"(~((CHOOSE zenon_Vvd:((zenon_Vvd \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&(?z_hcg={(zenon_Vvd[1]), (zenon_Vvd[2])}))) \\in prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a)))" (is "~?z_hdo")
    have z_Hdh_z_Hdn: "(~?z_hdc) == (~?z_hdo)" (is "?z_hdh == ?z_hdn")
    by (unfold prod_def)
    have z_Hdh: "?z_hdh"
    by (unfold z_Hdh_z_Hdn, fact z_Hdn)
    show FALSE
    by (rule notE [OF z_Hdh z_Hdc])
   next
    assume z_Hdp:"(?z_hde~=?z_hcg)"
    show FALSE
    by (rule zenon_eqsym [OF z_Hdd z_Hdp])
   qed
  qed
 qed
 have zenon_L2_: "(Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})) \\in a_CONSTANTunde_Surjectionunde_a(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))) ==> (~(Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})) \\in a_CONSTANTunde_Surjectionunde_a(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))) ==> FALSE" (is "?z_hdq ==> ?z_hds ==> FALSE")
 proof -
  assume z_Hdq:"?z_hdq"
  assume z_Hds:"?z_hds" (is "~?z_hdt")
  have z_Hdt_z_Hdq: "?z_hdt == ?z_hdq" (is "_ == _")
  by (unfold prod_def)
  have z_Hdt: "?z_hdt"
  by (unfold z_Hdt_z_Hdq, fact z_Hdq)
  show FALSE
  by (rule notE [OF z_Hds z_Hdt])
 qed
 assume z_Hc:"(~(Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})) \\in a_CONSTANTunde_Surjectionunde_a(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))))" (is "~?z_hdv")
 have z_Hdx_z_Hc: "(~(Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})) \\in a_CONSTANTunde_Surjectionunde_a(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))) == (~?z_hdv)" (is "?z_hdx == ?z_hc")
 by (unfold prod_def)
 have z_Hdx: "?z_hdx" (is "~?z_hdy")
 by (unfold z_Hdx_z_Hc, fact z_Hc)
 have z_Hds_z_Hdx: "(~(Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})) \\in a_CONSTANTunde_Surjectionunde_a(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))) == ?z_hdx" (is "?z_hds == _")
 by (unfold prod_def)
 have z_Hds: "?z_hds" (is "~?z_hdt")
 by (unfold z_Hds_z_Hdx, fact z_Hdx)
 have z_Hdz: "?z_hv(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))" (is "\\A x : ?z_hea(x)")
 by (rule zenon_all_0 [of "?z_hv" "Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>)", OF z_Hb])
 have z_Heb: "?z_hea(setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))" (is "?z_heb")
 by (rule zenon_all_0 [of "?z_hea" "setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))", OF z_Hdz])
 have z_Hec_z_Heb: "bAll(FuncSet(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))), (\<lambda>a_CONSTANTunde_Funde_a. (bAll(setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})), (\<lambda>a_CONSTANTunde_tunde_a. bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((a_CONSTANTunde_Funde_a[a_CONSTANTunde_sunde_a])=a_CONSTANTunde_tunde_a)))))=>(a_CONSTANTunde_Funde_a \\in a_CONSTANTunde_Surjectionunde_a(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))) == ?z_heb" (is "?z_hec == _")
 by (unfold prod_def)
 have z_Hec: "?z_hec"
 by (unfold z_Hec_z_Heb, fact z_Heb)
 have z_Hek_z_Hec: "(\\A x:((x \\in FuncSet(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))=>(bAll(setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})), (\<lambda>a_CONSTANTunde_tunde_a. bEx(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_sunde_a. ((x[a_CONSTANTunde_sunde_a])=a_CONSTANTunde_tunde_a)))))=>(x \\in a_CONSTANTunde_Surjectionunde_a(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))) == ?z_hec" (is "?z_hek == _")
 by (unfold bAll_def)
 have z_Hek: "?z_hek" (is "\\A x : ?z_hev(x)")
 by (unfold z_Hek_z_Hec, fact z_Hec)
 have z_Hew: "?z_hev(Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))" (is "?z_hex=>?z_hey")
 by (rule zenon_all_0 [of "?z_hev" "Fcn(prod(a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))", OF z_Hek])
 show FALSE
 proof (rule zenon_imply [OF z_Hew])
  assume z_Hez:"(~?z_hex)"
  have z_Hfa_z_Hez: "(~(Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})) \\in FuncSet(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))) == (~?z_hex)" (is "?z_hfa == ?z_hez")
  by (unfold prod_def)
  have z_Hfa: "?z_hfa" (is "~?z_hfb")
  by (unfold z_Hfa_z_Hez, fact z_Hez)
  show FALSE
  proof (rule zenon_notin_funcset [of "Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))" "Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>)" "setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))", OF z_Hfa])
   assume z_Hfc:"(~isAFcn(Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))" (is "~?z_hfd")
   show FALSE
   by (rule zenon_notisafcn_fcn [of "Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>)" "(\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})", OF z_Hfc])
  next
   assume z_Hfe:"(DOMAIN(Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))~=Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))" (is "?z_hff~=?z_hbk")
   have z_Hfg: "(?z_hbk~=?z_hbk)"
   by (rule zenon_domain_fcn_0 [of "(\<lambda>zenon_Vzwb. (zenon_Vzwb~=?z_hbk))" "?z_hbk" "(\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})", OF z_Hfe])
   show FALSE
   by (rule zenon_noteq [OF z_Hfg])
  next
   assume z_Hfk:"(~(\\A zenon_Vdoa:((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))" (is "~(\\A x : ?z_hfr(x))")
   have z_Hfs: "(\\E zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))" (is "\\E x : ?z_hfu(x)")
   by (rule zenon_notallex_0 [of "?z_hfr", OF z_Hfk])
   have z_Hfv: "?z_hfu((CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))))))" (is "~(?z_hfx=>?z_hfy)")
   by (rule zenon_ex_choose_0 [of "?z_hfu", OF z_Hfs])
   have z_Hfx: "?z_hfx"
   by (rule zenon_notimply_0 [OF z_Hfv])
   have z_Hfz: "(~?z_hfy)"
   by (rule zenon_notimply_1 [OF z_Hfv])
   have z_Hga: "(~(\\E zenon_Vjoa:((zenon_Vjoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))&((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[(CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))])={(zenon_Vjoa[1]), (zenon_Vjoa[2])}))))" (is "~(\\E x : ?z_hgk(x))")
   by (rule zenon_notin_setofall_0 [of "(Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[(CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))])" "Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>)" "(\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})", OF z_Hfz])
   have z_Hgl: "~?z_hgk((CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})))))))" (is "~(_&?z_hgm)")
   by (rule zenon_notex_0 [of "?z_hgk" "(CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))", OF z_Hga])
   show FALSE
   proof (rule zenon_notand [OF z_Hgl])
    assume z_Hgn:"(~?z_hfx)"
    show FALSE
    by (rule notE [OF z_Hgn z_Hfx])
   next
    assume z_Hgo:"((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[(CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))])~={((CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))[1]), ((CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))[2])})" (is "?z_hgg~=?z_hgp")
    show FALSE
    proof (rule zenon_fapplyfcn [of "(\<lambda>zenon_Vvwb. (zenon_Vvwb~=?z_hgp))" "Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>)" "(\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])})" "(CHOOSE zenon_Vdoa:(~((zenon_Vdoa \\in Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>))=>((Fcn(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))[zenon_Vdoa]) \\in setOfAll(Product(<<a_CONSTANTunde_Nodesunde_a, a_CONSTANTunde_Nodesunde_a>>), (\<lambda>a_CONSTANTunde_munde_a. {(a_CONSTANTunde_munde_a[1]), (a_CONSTANTunde_munde_a[2])}))))))", OF z_Hgo])
     assume z_Hgn:"(~?z_hfx)"
     show FALSE
     by (rule notE [OF z_Hgn z_Hfx])
    next
     assume z_Hgv:"(?z_hgp~=?z_hgp)"
     show FALSE
     by (rule zenon_noteq [OF z_Hgv])
    qed
   qed
  qed
 next
  assume z_Hey:"?z_hey" (is "?z_hx=>?z_hdq")
  show FALSE
  proof (rule zenon_imply [OF z_Hey])
   assume z_Hw:"(~?z_hx)"
   show FALSE
   by (rule zenon_L1_ [OF z_Hw])
  next
   assume z_Hdq:"?z_hdq"
   show FALSE
   by (rule zenon_L2_ [OF z_Hdq z_Hds])
  qed
 qed
qed
(* END-PROOF *)
ML_command {* writeln "*** TLAPS EXIT 9"; *} qed
lemma ob'1:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
shows "(\<forall>a_CONSTANTunde_Nodesunde_a : ((\<forall> a_CONSTANTunde_munde_a \<in> (a_CONSTANTunde_Nodesunde_a) : (\<forall> a_CONSTANTunde_nunde_a \<in> (a_CONSTANTunde_Nodesunde_a) : ((({(a_CONSTANTunde_munde_a), (a_CONSTANTunde_nunde_a)}) \<in> (setOfAll((((a_CONSTANTunde_Nodesunde_a) \<times> (a_CONSTANTunde_Nodesunde_a))), %a_CONSTANTunde_munde_a_1. ({(fapply ((a_CONSTANTunde_munde_a_1), ((Succ[0])))), (fapply ((a_CONSTANTunde_munde_a_1), ((Succ[Succ[0]]))))}))))))) & (\<forall> a_CONSTANTunde_eunde_a \<in> (setOfAll((((a_CONSTANTunde_Nodesunde_a) \<times> (a_CONSTANTunde_Nodesunde_a))), %a_CONSTANTunde_munde_a. ({(fapply ((a_CONSTANTunde_munde_a), ((Succ[0])))), (fapply ((a_CONSTANTunde_munde_a), ((Succ[Succ[0]]))))}))) : (\<exists> a_CONSTANTunde_munde_a \<in> (a_CONSTANTunde_Nodesunde_a) : (\<exists> a_CONSTANTunde_nunde_a \<in> (a_CONSTANTunde_Nodesunde_a) : (((a_CONSTANTunde_eunde_a) = ({(a_CONSTANTunde_munde_a), (a_CONSTANTunde_nunde_a)}))))))))"(is "PROP ?ob'1")
proof -
ML_command {* writeln "*** TLAPS ENTER 1"; *}
show "PROP ?ob'1"
using assms by force
ML_command {* writeln "*** TLAPS EXIT 1"; *} qed
lemma ob'110:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
(* usable definition CONSTANT_Edges_ suppressed *)
(* usable definition CONSTANT_NonLoopEdges_ suppressed *)
(* usable definition CONSTANT_SimpleGraphs_ suppressed *)
(* usable definition CONSTANT_Degree_ suppressed *)
fixes a_CONSTANTunde_Nodesunde_a
assumes v'155: "((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a))))"
assumes v'156: "((greater (((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Nodesunde_a)))), ((Succ[0])))))"
fixes a_CONSTANTunde_Gunde_a
assumes a_CONSTANTunde_Gunde_a_in : "(a_CONSTANTunde_Gunde_a \<in> ((a_CONSTANTunde_SimpleGraphsunde_a ((a_CONSTANTunde_Nodesunde_a)))))"
assumes v'175: "(((a_CONSTANTunde_IsFiniteSetunde_a ((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))))) & ((((a_CONSTANTunde_Cardinalityunde_a ((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))))) \<in> (Nat))))"
assumes v'176: "((geq (((a_CONSTANTunde_Cardinalityunde_a ((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))))), ((Succ[0])))))"
assumes v'177: "((([ a_CONSTANTunde_nunde_a \<in> (((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))  \<mapsto> ((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a))))]) \<in> ([(((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) \<rightarrow> ((isa_peri_peri_a (((Succ[0])), ((arith_add (((a_CONSTANTunde_Cardinalityunde_a ((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))))), ((minus (((Succ[0])))))))))))])))"
assumes v'178: "(((a_CONSTANTunde_IsFiniteSetunde_a (((isa_peri_peri_a (((Succ[0])), ((arith_add (((a_CONSTANTunde_Cardinalityunde_a ((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))))), ((minus (((Succ[0])))))))))))))) & ((less (((a_CONSTANTunde_Cardinalityunde_a (((isa_peri_peri_a (((Succ[0])), ((arith_add (((a_CONSTANTunde_Cardinalityunde_a ((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))))), ((minus (((Succ[0])))))))))))))), ((a_CONSTANTunde_Cardinalityunde_a ((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))))))))))"
assumes v'179: "((\<And> a_CONSTANTunde_Sunde_a :: c. (((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Sunde_a)))) \<Longrightarrow> (\<And> a_CONSTANTunde_Tunde_a :: c. (((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Tunde_a)))) \<Longrightarrow> (((less (((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Tunde_a)))), ((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Sunde_a))))))) \<Longrightarrow> (\<And> a_CONSTANTunde_funde_a :: c. a_CONSTANTunde_funde_a \<in> ([(a_CONSTANTunde_Sunde_a) \<rightarrow> (a_CONSTANTunde_Tunde_a)]) \<Longrightarrow> (\<exists> a_CONSTANTunde_xunde_a \<in> (a_CONSTANTunde_Sunde_a) : (\<exists> a_CONSTANTunde_yunde_a \<in> (a_CONSTANTunde_Sunde_a) : (((((a_CONSTANTunde_xunde_a) \<noteq> (a_CONSTANTunde_yunde_a))) \<and> (((fapply ((a_CONSTANTunde_funde_a), (a_CONSTANTunde_xunde_a))) = (fapply ((a_CONSTANTunde_funde_a), (a_CONSTANTunde_yunde_a))))))))))))))))"
shows "(\<exists> a_CONSTANTunde_munde_a \<in> (((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) : (\<exists> a_CONSTANTunde_nunde_a \<in> (((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) : (((((a_CONSTANTunde_munde_a) \<noteq> (a_CONSTANTunde_nunde_a))) \<and> (((fapply (([ a_CONSTANTunde_nunde_a_1 \<in> (((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a_1. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))  \<mapsto> ((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a))))]), (a_CONSTANTunde_munde_a))) = (fapply (([ a_CONSTANTunde_nunde_a_1 \<in> (((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a_1. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a)))) = ((0))))))))  \<mapsto> ((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a))))]), (a_CONSTANTunde_nunde_a)))))))))"(is "PROP ?ob'110")
proof -
ML_command {* writeln "*** TLAPS ENTER 110"; *}
show "PROP ?ob'110"
using assms by auto
ML_command {* writeln "*** TLAPS EXIT 110"; *} qed
lemma ob'100:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
(* usable definition CONSTANT_Edges_ suppressed *)
(* usable definition CONSTANT_NonLoopEdges_ suppressed *)
(* usable definition CONSTANT_SimpleGraphs_ suppressed *)
(* usable definition CONSTANT_Degree_ suppressed *)
fixes a_CONSTANTunde_Nodesunde_a
assumes v'155: "((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a))))"
assumes v'156: "((greater (((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Nodesunde_a)))), ((Succ[0])))))"
fixes a_CONSTANTunde_Gunde_a
assumes a_CONSTANTunde_Gunde_a_in : "(a_CONSTANTunde_Gunde_a \<in> ((a_CONSTANTunde_SimpleGraphsunde_a ((a_CONSTANTunde_Nodesunde_a)))))"
fixes a_CONSTANTunde_nunde_a
assumes a_CONSTANTunde_nunde_a_in : "(a_CONSTANTunde_nunde_a \<in> (((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))))"
assumes v'181: "((\<And> a_CONSTANTunde_Sunde_a :: c. (\<And> a_CONSTANTunde_Tunde_a :: c. (\<And> a_CONSTANTunde_Funde_a :: c. (((((a_CONSTANTunde_Funde_a) \<in> ((a_CONSTANTunde_Injectionunde_a ((a_CONSTANTunde_Sunde_a), (a_CONSTANTunde_Tunde_a)))))) | (((((a_CONSTANTunde_Funde_a) \<in> ([(a_CONSTANTunde_Sunde_a) \<rightarrow> (a_CONSTANTunde_Tunde_a)]))) \<and> (\<forall> a_CONSTANTunde_aunde_a \<in> (a_CONSTANTunde_Sunde_a) : (\<forall> a_CONSTANTunde_bunde_a \<in> (a_CONSTANTunde_Sunde_a) : (((((fapply ((a_CONSTANTunde_Funde_a), (a_CONSTANTunde_aunde_a))) = (fapply ((a_CONSTANTunde_Funde_a), (a_CONSTANTunde_bunde_a))))) \<Rightarrow> (((a_CONSTANTunde_aunde_a) = (a_CONSTANTunde_bunde_a)))))))))) \<Longrightarrow> (((((a_CONSTANTunde_Funde_a) \<in> ((a_CONSTANTunde_Surjectionunde_a ((a_CONSTANTunde_Sunde_a), (a_CONSTANTunde_Tunde_a)))))) | (((((a_CONSTANTunde_Funde_a) \<in> ([(a_CONSTANTunde_Sunde_a) \<rightarrow> (a_CONSTANTunde_Tunde_a)]))) \<and> (\<forall> a_CONSTANTunde_tunde_a \<in> (a_CONSTANTunde_Tunde_a) : (\<exists> a_CONSTANTunde_sunde_a \<in> (a_CONSTANTunde_Sunde_a) : (((fapply ((a_CONSTANTunde_Funde_a), (a_CONSTANTunde_sunde_a))) = (a_CONSTANTunde_tunde_a)))))))) \<Longrightarrow> (((a_CONSTANTunde_Funde_a) \<in> ((a_CONSTANTunde_Bijectionunde_a ((a_CONSTANTunde_Sunde_a), (a_CONSTANTunde_Tunde_a))))))))))))"
shows "((([ a_CONSTANTunde_munde_a \<in> (((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a_1. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) \\ ({(a_CONSTANTunde_nunde_a)})))  \<mapsto> ({(a_CONSTANTunde_munde_a), (a_CONSTANTunde_nunde_a)})]) \<in> ((a_CONSTANTunde_Bijectionunde_a ((((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a_1. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) \\ ({(a_CONSTANTunde_nunde_a)}))), (setOfAll((((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a_1. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) \\ ({(a_CONSTANTunde_nunde_a)}))), %a_CONSTANTunde_munde_a. ({(a_CONSTANTunde_munde_a), (a_CONSTANTunde_nunde_a)}))))))))"(is "PROP ?ob'100")
proof -
ML_command {* writeln "*** TLAPS ENTER 100"; *}
show "PROP ?ob'100"

(* BEGIN ZENON INPUT
;; file=.tlacache/GraphTheorem.tlaps/tlapm_330011.znn; PATH='/tmp/tlaps/inst/bin:/home/tarek/.kimi-code/bin:/home/tarek/.local/share/swiftly/bin:/home/tarek/.rbenv/shims:/run/user/1000/fnm_multishells/98560_1789588384174/bin:/home/tarek/.local/share/fnm:/home/tarek/.rbenv/shims:/home/tarek/.rbenv/bin:/home/tarek/.local/bin:/home/tarek/.cargo/bin:/run/user/1000/fnm_multishells/581_1789534734414/bin:/home/tarek/.local/share/fnm:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games:/usr/local/games:/usr/lib/wsl/lib:/mnt/c/Program Files/coreutils/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.3/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.2/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.2/bin:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.1/bin/x64:/mnt/c/Program Files/NVIDIA GPU Computing Toolkit/CUDA/v13.1/bin:/mnt/c/WINDOWS/system32:/mnt/c/WINDOWS:/mnt/c/WINDOWS/System32/Wbem:/mnt/c/WINDOWS/System32/WindowsPowerShell/v1.0/:/mnt/c/WINDOWS/System32/OpenSSH/:/mnt/c/Program Files/Microsoft SQL Server/170/Tools/Binn/:/mnt/c/Program Files/Microsoft SQL Server/Client SDK/ODBC/170/Tools/Binn/:/mnt/c/Program Files/NVIDIA Corporation/Nsight Compute 2026.2.0/:/mnt/c/Program Files (x86)/Windows Kits/10/Windows Performance Toolkit/:/mnt/c/Program Files/NVIDIA Corporation/NVIDIA App/NvDLISR:/mnt/c/Program Files/dotnet/:/mnt/c/Program Files/CMake/bin:/mnt/c/Program Files/Docker/Docker/resources/bin:/mnt/c/Program Files/Git/cmd:/mnt/c/Program Files/GitHub CLI/:/mnt/c/Users/tarek/.kimi-code/bin:/mnt/c/Users/tarek/AppData/Local/Microsoft/WindowsApps:/mnt/c/Users/tarek/AppData/Local/Programs/Microsoft VS Code/bin:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/Fastfetch-cli.Fastfetch_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/Programs/codecrafters:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/SST.opencode_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/AppData/Local/Programs/Ollama:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/Schniz.fnm_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Packages/sinelaw.fresh-editor_Microsoft.Winget.Source_8wekyb3d8bbwe:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/Microsoft/WinGet/Links:/mnt/c/Users/tarek/AppData/Local/Programs/Zed/bin:/mnt/c/Users/tarek/.dotnet/tools:/mnt/c/Users/tarek/AppData/Local/PowerToys/DSCModules/:/snap/bin:/tmp/tlaps/inst/bin:/tmp/tlaps/inst/lib/tlaps/bin'; zenon -p0 -x tla -oisar -max-time 1d "$file" >.tlacache/GraphTheorem.tlaps/tlapm_330011.znn.out
;; obligation #100
$hyp "v'155" (a_CONSTANTunde_IsFiniteSetunde_a a_CONSTANTunde_Nodesunde_a)
$hyp "v'156" (arith.lt (TLA.fapply TLA.Succ 0)
(a_CONSTANTunde_Cardinalityunde_a a_CONSTANTunde_Nodesunde_a))
$hyp "a_CONSTANTunde_Gunde_a_in" (TLA.in a_CONSTANTunde_Gunde_a (a_CONSTANTunde_SimpleGraphsunde_a a_CONSTANTunde_Nodesunde_a))
$hyp "a_CONSTANTunde_nunde_a_in" (TLA.in a_CONSTANTunde_nunde_a (TLA.setminus a_CONSTANTunde_Nodesunde_a
(TLA.subsetOf a_CONSTANTunde_Nodesunde_a ((a_CONSTANTunde_nunde_a) (= (a_CONSTANTunde_Degreeunde_a a_CONSTANTunde_nunde_a
a_CONSTANTunde_Gunde_a)
0)))))
$hyp "v'181" (A. ((a_CONSTANTunde_Sunde_a) (A. ((a_CONSTANTunde_Tunde_a) (A. ((a_CONSTANTunde_Funde_a) (=> (\/ (TLA.in a_CONSTANTunde_Funde_a
(a_CONSTANTunde_Injectionunde_a a_CONSTANTunde_Sunde_a
a_CONSTANTunde_Tunde_a)) (/\ (TLA.in a_CONSTANTunde_Funde_a
(TLA.FuncSet a_CONSTANTunde_Sunde_a a_CONSTANTunde_Tunde_a))
(TLA.bAll a_CONSTANTunde_Sunde_a ((a_CONSTANTunde_aunde_a) (TLA.bAll a_CONSTANTunde_Sunde_a ((a_CONSTANTunde_bunde_a) (=> (= (TLA.fapply a_CONSTANTunde_Funde_a a_CONSTANTunde_aunde_a)
(TLA.fapply a_CONSTANTunde_Funde_a a_CONSTANTunde_bunde_a))
(= a_CONSTANTunde_aunde_a
a_CONSTANTunde_bunde_a)))))))) (=> (\/ (TLA.in a_CONSTANTunde_Funde_a
(a_CONSTANTunde_Surjectionunde_a a_CONSTANTunde_Sunde_a
a_CONSTANTunde_Tunde_a)) (/\ (TLA.in a_CONSTANTunde_Funde_a
(TLA.FuncSet a_CONSTANTunde_Sunde_a a_CONSTANTunde_Tunde_a))
(TLA.bAll a_CONSTANTunde_Tunde_a ((a_CONSTANTunde_tunde_a) (TLA.bEx a_CONSTANTunde_Sunde_a ((a_CONSTANTunde_sunde_a) (= (TLA.fapply a_CONSTANTunde_Funde_a a_CONSTANTunde_sunde_a)
a_CONSTANTunde_tunde_a))))))) (TLA.in a_CONSTANTunde_Funde_a
(a_CONSTANTunde_Bijectionunde_a a_CONSTANTunde_Sunde_a
a_CONSTANTunde_Tunde_a))))))))))
$goal (TLA.in (TLA.Fcn (TLA.setminus (TLA.setminus a_CONSTANTunde_Nodesunde_a
(TLA.subsetOf a_CONSTANTunde_Nodesunde_a ((a_CONSTANTunde_nunde_a_1) (= (a_CONSTANTunde_Degreeunde_a a_CONSTANTunde_nunde_a_1
a_CONSTANTunde_Gunde_a) 0))))
(TLA.set a_CONSTANTunde_nunde_a)) ((a_CONSTANTunde_munde_a) (TLA.set a_CONSTANTunde_munde_a a_CONSTANTunde_nunde_a)))
(a_CONSTANTunde_Bijectionunde_a (TLA.setminus (TLA.setminus a_CONSTANTunde_Nodesunde_a
(TLA.subsetOf a_CONSTANTunde_Nodesunde_a ((a_CONSTANTunde_nunde_a_1) (= (a_CONSTANTunde_Degreeunde_a a_CONSTANTunde_nunde_a_1
a_CONSTANTunde_Gunde_a) 0)))) (TLA.set a_CONSTANTunde_nunde_a))
(TLA.setOfAll (TLA.setminus (TLA.setminus a_CONSTANTunde_Nodesunde_a
(TLA.subsetOf a_CONSTANTunde_Nodesunde_a ((a_CONSTANTunde_nunde_a_1) (= (a_CONSTANTunde_Degreeunde_a a_CONSTANTunde_nunde_a_1
a_CONSTANTunde_Gunde_a) 0))))
(TLA.set a_CONSTANTunde_nunde_a)) ((a_CONSTANTunde_munde_a) (TLA.set a_CONSTANTunde_munde_a a_CONSTANTunde_nunde_a)))))
END ZENON  INPUT *)
(* PROOF-FOUND *)
(* BEGIN-PROOF *)
proof (rule zenon_nnpp)
 have z_He:"(\\A a_CONSTANTunde_Sunde_a:(\\A a_CONSTANTunde_Tunde_a:(\\A a_CONSTANTunde_Funde_a:(((a_CONSTANTunde_Funde_a \\in a_CONSTANTunde_Injectionunde_a(a_CONSTANTunde_Sunde_a, a_CONSTANTunde_Tunde_a))|((a_CONSTANTunde_Funde_a \\in FuncSet(a_CONSTANTunde_Sunde_a, a_CONSTANTunde_Tunde_a))&bAll(a_CONSTANTunde_Sunde_a, (\<lambda>a_CONSTANTunde_aunde_a. bAll(a_CONSTANTunde_Sunde_a, (\<lambda>a_CONSTANTunde_bunde_a. (((a_CONSTANTunde_Funde_a[a_CONSTANTunde_aunde_a])=(a_CONSTANTunde_Funde_a[a_CONSTANTunde_bunde_a]))=>(a_CONSTANTunde_aunde_a=a_CONSTANTunde_bunde_a))))))))=>(((a_CONSTANTunde_Funde_a \\in a_CONSTANTunde_Surjectionunde_a(a_CONSTANTunde_Sunde_a, a_CONSTANTunde_Tunde_a))|((a_CONSTANTunde_Funde_a \\in FuncSet(a_CONSTANTunde_Sunde_a, a_CONSTANTunde_Tunde_a))&bAll(a_CONSTANTunde_Tunde_a, (\<lambda>a_CONSTANTunde_tunde_a. bEx(a_CONSTANTunde_Sunde_a, (\<lambda>a_CONSTANTunde_sunde_a. ((a_CONSTANTunde_Funde_a[a_CONSTANTunde_sunde_a])=a_CONSTANTunde_tunde_a)))))))=>(a_CONSTANTunde_Funde_a \\in a_CONSTANTunde_Bijectionunde_a(a_CONSTANTunde_Sunde_a, a_CONSTANTunde_Tunde_a)))))))" (is "\\A x : ?z_hbs(x)")
 using v'181 by blast
 have zenon_L1_: "(~(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})) \\in FuncSet(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))) ==> FALSE" (is "?z_hbt ==> FALSE")
 proof -
  assume z_Hbt:"?z_hbt" (is "~?z_hbu")
  show FALSE
  proof (rule zenon_notin_funcset [of "Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))", OF z_Hbt])
   assume z_Hcm:"(~isAFcn(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))" (is "~?z_hcn")
   show FALSE
   by (rule zenon_notisafcn_fcn [of "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})", OF z_Hcm])
  next
   assume z_Hco:"(DOMAIN(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))~=((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))" (is "?z_hcp~=?z_hbw")
   have z_Hcq: "(?z_hbw~=?z_hbw)"
   by (rule zenon_domain_fcn_0 [of "(\<lambda>zenon_Vuda. (zenon_Vuda~=?z_hbw))" "?z_hbw" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})", OF z_Hco])
   show FALSE
   by (rule zenon_noteq [OF z_Hcq])
  next
   assume z_Hcu:"(~(\\A zenon_Vfoa:((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))))" (is "~(\\A x : ?z_hdb(x))")
   have z_Hdc: "(\\E zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))))" (is "\\E x : ?z_hde(x)")
   by (rule zenon_notallex_0 [of "?z_hdb", OF z_Hcu])
   have z_Hdf: "?z_hde((CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))))))" (is "~(?z_hdh=>?z_hdi)")
   by (rule zenon_ex_choose_0 [of "?z_hde", OF z_Hdc])
   have z_Hdh: "?z_hdh"
   by (rule zenon_notimply_0 [OF z_Hdf])
   have z_Hdj: "(~?z_hdi)"
   by (rule zenon_notimply_1 [OF z_Hdf])
   have z_Hdk: "(~(\\E zenon_Vloa:((zenon_Vloa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[(CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))))])={zenon_Vloa, a_CONSTANTunde_nunde_a}))))" (is "~(\\E x : ?z_hds(x))")
   by (rule zenon_notin_setofall_0 [of "(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[(CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))))])" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})", OF z_Hdj])
   have z_Hdt: "~?z_hds((CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))))))" (is "~(_&?z_hdu)")
   by (rule zenon_notex_0 [of "?z_hds" "(CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))))", OF z_Hdk])
   show FALSE
   proof (rule zenon_notand [OF z_Hdt])
    assume z_Hdv:"(~?z_hdh)"
    show FALSE
    by (rule notE [OF z_Hdv z_Hdh])
   next
    assume z_Hdw:"((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[(CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))))])~={(CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))))), a_CONSTANTunde_nunde_a})" (is "?z_hdq~=?z_hdx")
    show FALSE
    proof (rule zenon_fapplyfcn [of "(\<lambda>zenon_Vwab. (zenon_Vwab~=?z_hdx))" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})" "(CHOOSE zenon_Vfoa:(~((zenon_Vfoa \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[zenon_Vfoa]) \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))))))", OF z_Hdw])
     assume z_Hdv:"(~?z_hdh)"
     show FALSE
     by (rule notE [OF z_Hdv z_Hdh])
    next
     assume z_Heb:"(?z_hdx~=?z_hdx)"
     show FALSE
     by (rule zenon_noteq [OF z_Heb])
    qed
   qed
  qed
 qed
 assume z_Hf:"(~(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})) \\in a_CONSTANTunde_Bijectionunde_a(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))))" (is "~?z_hec")
 have z_Hee: "?z_hbs(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))" (is "\\A x : ?z_hef(x)")
 by (rule zenon_all_0 [of "?z_hbs" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})", OF z_He])
 have z_Heg: "?z_hef(setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))" (is "\\A x : ?z_heh(x)")
 by (rule zenon_all_0 [of "?z_hef" "setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))", OF z_Hee])
 have z_Hei: "?z_heh(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))" (is "?z_hej=>?z_hek")
 by (rule zenon_all_0 [of "?z_heh" "Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))", OF z_Heg])
 show FALSE
 proof (rule zenon_imply [OF z_Hei])
  assume z_Hel:"(~?z_hej)" (is "~(?z_hem|?z_hen)")
  have z_Heo: "(~?z_hen)" (is "~(?z_hbu&?z_hep)")
  by (rule zenon_notor_1 [OF z_Hel])
  show FALSE
  proof (rule zenon_notand [OF z_Heo])
   assume z_Hbt:"(~?z_hbu)"
   show FALSE
   by (rule zenon_L1_ [OF z_Hbt])
  next
   assume z_Heq:"(~?z_hep)"
   have z_Her_z_Heq: "(~(\\A x:((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>bAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_bunde_a. (((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_bunde_a]))=>(x=a_CONSTANTunde_bunde_a))))))) == (~?z_hep)" (is "?z_her == ?z_heq")
   by (unfold bAll_def)
   have z_Her: "?z_her" (is "~(\\A x : ?z_hfd(x))")
   by (unfold z_Her_z_Heq, fact z_Heq)
   have z_Hfe: "(\\E x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>bAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_bunde_a. (((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_bunde_a]))=>(x=a_CONSTANTunde_bunde_a)))))))" (is "\\E x : ?z_hfg(x)")
   by (rule zenon_notallex_0 [of "?z_hfd", OF z_Her])
   have z_Hfh: "?z_hfg((CHOOSE x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>bAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_bunde_a. (((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_bunde_a]))=>(x=a_CONSTANTunde_bunde_a))))))))" (is "~(?z_hfj=>?z_hfk)")
   by (rule zenon_ex_choose_0 [of "?z_hfg", OF z_Hfe])
   have z_Hfj: "?z_hfj"
   by (rule zenon_notimply_0 [OF z_Hfh])
   have z_Hfl: "(~?z_hfk)"
   by (rule zenon_notimply_1 [OF z_Hfh])
   have z_Hfm: "(~((CHOOSE x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>bAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_bunde_a. (((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_bunde_a]))=>(x=a_CONSTANTunde_bunde_a))))))) \\in {a_CONSTANTunde_nunde_a}))" (is "~?z_hfn")
   by (rule zenon_in_setminus_1 [of "(CHOOSE x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>bAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_bunde_a. (((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_bunde_a]))=>(x=a_CONSTANTunde_bunde_a)))))))" "(a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0))))" "{a_CONSTANTunde_nunde_a}", OF z_Hfj])
   have z_Hfo: "((CHOOSE x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>bAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_bunde_a. (((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_bunde_a]))=>(x=a_CONSTANTunde_bunde_a)))))))~=a_CONSTANTunde_nunde_a)" (is "?z_hfi~=_")
   by (rule zenon_notin_addElt_0 [of "?z_hfi" "a_CONSTANTunde_nunde_a" "{}", OF z_Hfm])
   have z_Hfq_z_Hfl: "(~(\\A x:((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>(((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[?z_hfi])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x]))=>(?z_hfi=x))))) == (~?z_hfk)" (is "?z_hfq == ?z_hfl")
   by (unfold bAll_def)
   have z_Hfq: "?z_hfq" (is "~(\\A x : ?z_hfx(x))")
   by (unfold z_Hfq_z_Hfl, fact z_Hfl)
   have z_Hfy: "(\\E x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>(((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[?z_hfi])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x]))=>(?z_hfi=x)))))" (is "\\E x : ?z_hga(x)")
   by (rule zenon_notallex_0 [of "?z_hfx", OF z_Hfq])
   have z_Hgb: "?z_hga((CHOOSE x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>(((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[?z_hfi])=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x]))=>(?z_hfi=x))))))" (is "~(?z_hgd=>?z_hge)")
   by (rule zenon_ex_choose_0 [of "?z_hga", OF z_Hfy])
   have z_Hgd: "?z_hgd"
   by (rule zenon_notimply_0 [OF z_Hgb])
   have z_Hgf: "(~?z_hge)" (is "~(?z_hgg=>?z_hgh)")
   by (rule zenon_notimply_1 [OF z_Hgb])
   have z_Hgg: "?z_hgg" (is "?z_hfv=?z_hgi")
   by (rule zenon_notimply_0 [OF z_Hgf])
   have z_Hgj: "(?z_hfi~=(CHOOSE x:(~((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))=>((?z_hfv=(Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x]))=>(?z_hfi=x))))))" (is "_~=?z_hgc")
   by (rule zenon_notimply_1 [OF z_Hgf])
   show FALSE
   proof (rule zenon_fapplyfcn [of "(\<lambda>zenon_Vac. (zenon_Vac=?z_hgi))" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})" "?z_hfi", OF z_Hgg])
    assume z_Hgn:"(~?z_hfj)"
    show FALSE
    by (rule notE [OF z_Hgn z_Hfj])
   next
    assume z_Hgo:"({?z_hfi, a_CONSTANTunde_nunde_a}=?z_hgi)" (is "?z_hgp=_")
    show FALSE
    proof (rule zenon_fapplyfcn [of "(\<lambda>zenon_Voc. (?z_hgp=zenon_Voc))" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})" "?z_hgc", OF z_Hgo])
     assume z_Hgt:"(~?z_hgd)"
     show FALSE
     by (rule notE [OF z_Hgt z_Hgd])
    next
     assume z_Hgu:"(?z_hgp={?z_hgc, a_CONSTANTunde_nunde_a})" (is "_=?z_hgv")
     have z_Hgw: "(\\A zenon_Vpc:((zenon_Vpc \\in ?z_hgp)<=>(zenon_Vpc \\in ?z_hgi)))" (is "\\A x : ?z_hhb(x)")
     by (rule zenon_setequal_0 [of "?z_hgp" "?z_hgi", OF z_Hgo])
     have z_Hhc: "?z_hhb(?z_hfi)" (is "?z_hhd<=>?z_hhe")
     by (rule zenon_all_0 [of "?z_hhb" "?z_hfi", OF z_Hgw])
     show FALSE
     proof (rule zenon_equiv [OF z_Hhc])
      assume z_Hhf:"(~?z_hhd)"
      assume z_Hhg:"(~?z_hhe)"
      have z_Hhh: "(?z_hfi~=?z_hfi)"
      by (rule zenon_notin_addElt_0 [of "?z_hfi" "?z_hfi" "{a_CONSTANTunde_nunde_a}", OF z_Hhf])
      show FALSE
      by (rule zenon_noteq [OF z_Hhh])
     next
      assume z_Hhd:"?z_hhd"
      assume z_Hhe:"?z_hhe"
      have z_Hhi: "(?z_hfi \\in ?z_hgv)" (is "?z_hhi")
      by (rule subst [where P="(\<lambda>zenon_Vuna. (?z_hfi \\in zenon_Vuna))", OF z_Hgu z_Hhd])
      show FALSE
      proof (rule zenon_in_addElt [of "?z_hfi" "?z_hgc" "{a_CONSTANTunde_nunde_a}", OF z_Hhi])
       assume z_Hgh:"?z_hgh"
       show FALSE
       by (rule notE [OF z_Hgj z_Hgh])
      next
       assume z_Hfn:"?z_hfn"
       show FALSE
       proof (rule zenon_in_addElt [of "?z_hfi" "a_CONSTANTunde_nunde_a" "{}", OF z_Hfn])
        assume z_Hhm:"(?z_hfi=a_CONSTANTunde_nunde_a)"
        show FALSE
        by (rule notE [OF z_Hfo z_Hhm])
       next
        assume z_Hhn:"(?z_hfi \\in {})" (is "?z_hhn")
        show FALSE
        by (rule zenon_in_emptyset [of "?z_hfi", OF z_Hhn])
       qed
      qed
     qed
    qed
   qed
  qed
 next
  assume z_Hek:"?z_hek" (is "?z_hho=>_")
  show FALSE
  proof (rule zenon_imply [OF z_Hek])
   assume z_Hhp:"(~?z_hho)" (is "~(?z_hhq|?z_hhr)")
   have z_Hhs: "(~?z_hhr)" (is "~(?z_hbu&?z_hht)")
   by (rule zenon_notor_1 [OF z_Hhp])
   show FALSE
   proof (rule zenon_notand [OF z_Hhs])
    assume z_Hbt:"(~?z_hbu)"
    show FALSE
    by (rule zenon_L1_ [OF z_Hbt])
   next
    assume z_Hhu:"(~?z_hht)"
    have z_Hhv_z_Hhu: "(~(\\A x:((x \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))=>bEx(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_sunde_a])=x)))))) == (~?z_hht)" (is "?z_hhv == ?z_hhu")
    by (unfold bAll_def)
    have z_Hhv: "?z_hhv" (is "~(\\A x : ?z_hid(x))")
    by (unfold z_Hhv_z_Hhu, fact z_Hhu)
    have z_Hie: "(\\E x:(~((x \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))=>bEx(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_sunde_a])=x))))))" (is "\\E x : ?z_hig(x)")
    by (rule zenon_notallex_0 [of "?z_hid", OF z_Hhv])
    have z_Hih: "?z_hig((CHOOSE x:(~((x \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))=>bEx(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_sunde_a])=x)))))))" (is "~(?z_hij=>?z_hik)")
    by (rule zenon_ex_choose_0 [of "?z_hig", OF z_Hie])
    have z_Hij: "?z_hij"
    by (rule zenon_notimply_0 [OF z_Hih])
    have z_Hil: "(~?z_hik)"
    by (rule zenon_notimply_1 [OF z_Hih])
    have z_Him_z_Hil: "(~(\\E x:((x \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[x])=(CHOOSE x:(~((x \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))=>bEx(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_sunde_a])=x)))))))))) == (~?z_hik)" (is "?z_him == ?z_hil")
    by (unfold bEx_def)
    have z_Him: "?z_him" (is "~(\\E x : ?z_hiq(x))")
    by (unfold z_Him_z_Hil, fact z_Hil)
    have z_Hir: "(\\E zenon_Vnbb:((zenon_Vnbb \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&((CHOOSE x:(~((x \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))=>bEx(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_sunde_a])=x))))))={zenon_Vnbb, a_CONSTANTunde_nunde_a})))" (is "\\E x : ?z_hix(x)")
    by (rule zenon_in_setofall_0 [of "(CHOOSE x:(~((x \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))=>bEx(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_sunde_a])=x))))))" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})", OF z_Hij])
    have z_Hiy: "?z_hix((CHOOSE zenon_Vnbb:((zenon_Vnbb \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&((CHOOSE x:(~((x \\in setOfAll(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})))=>bEx(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_sunde_a. ((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[a_CONSTANTunde_sunde_a])=x))))))={zenon_Vnbb, a_CONSTANTunde_nunde_a}))))" (is "?z_hja&?z_hjb")
    by (rule zenon_ex_choose_0 [of "?z_hix", OF z_Hir])
    have z_Hja: "?z_hja"
    by (rule zenon_and_0 [OF z_Hiy])
    have z_Hjb: "?z_hjb" (is "?z_hii=?z_hjc")
    by (rule zenon_and_1 [OF z_Hiy])
    have z_Hjd: "~?z_hiq((CHOOSE zenon_Vnbb:((zenon_Vnbb \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&(?z_hii={zenon_Vnbb, a_CONSTANTunde_nunde_a}))))" (is "~(_&?z_hje)")
    by (rule zenon_notex_0 [of "?z_hiq" "(CHOOSE zenon_Vnbb:((zenon_Vnbb \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&(?z_hii={zenon_Vnbb, a_CONSTANTunde_nunde_a})))", OF z_Him])
    show FALSE
    proof (rule zenon_notand [OF z_Hjd])
     assume z_Hjf:"(~?z_hja)"
     show FALSE
     by (rule notE [OF z_Hjf z_Hja])
    next
     assume z_Hjg:"((Fcn(((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}), (\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a}))[(CHOOSE zenon_Vnbb:((zenon_Vnbb \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&(?z_hii={zenon_Vnbb, a_CONSTANTunde_nunde_a})))])~=?z_hii)" (is "?z_hjh~=_")
     show FALSE
     proof (rule zenon_fapplyfcn [of "(\<lambda>zenon_Vqbb. (zenon_Vqbb~=?z_hii))" "((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a})" "(\<lambda>a_CONSTANTunde_munde_a. {a_CONSTANTunde_munde_a, a_CONSTANTunde_nunde_a})" "(CHOOSE zenon_Vnbb:((zenon_Vnbb \\in ((a_CONSTANTunde_Nodesunde_a \\ subsetOf(a_CONSTANTunde_Nodesunde_a, (\<lambda>a_CONSTANTunde_nunde_a. (a_CONSTANTunde_Degreeunde_a(a_CONSTANTunde_nunde_a, a_CONSTANTunde_Gunde_a)=0)))) \\ {a_CONSTANTunde_nunde_a}))&(?z_hii={zenon_Vnbb, a_CONSTANTunde_nunde_a})))", OF z_Hjg])
      assume z_Hjf:"(~?z_hja)"
      show FALSE
      by (rule notE [OF z_Hjf z_Hja])
     next
      assume z_Hjl:"(?z_hjc~=?z_hii)"
      show FALSE
      by (rule zenon_eqsym [OF z_Hjb z_Hjl])
     qed
    qed
   qed
  next
   assume z_Hec:"?z_hec"
   show FALSE
   by (rule notE [OF z_Hf z_Hec])
  qed
 qed
qed
(* END-PROOF *)
ML_command {* writeln "*** TLAPS EXIT 100"; *} qed
lemma ob'93:
(* usable definition CONSTANT_IsFiniteSet_ suppressed *)
(* usable definition CONSTANT_Cardinality_ suppressed *)
(* usable definition CONSTANT_Restrict_ suppressed *)
(* usable definition CONSTANT_Range_ suppressed *)
(* usable definition CONSTANT_Inverse_ suppressed *)
(* usable definition CONSTANT_IsInjective_ suppressed *)
(* usable definition CONSTANT_Injection_ suppressed *)
(* usable definition CONSTANT_Surjection_ suppressed *)
(* usable definition CONSTANT_Bijection_ suppressed *)
(* usable definition CONSTANT_ExistsInjection_ suppressed *)
(* usable definition CONSTANT_ExistsSurjection_ suppressed *)
(* usable definition CONSTANT_ExistsBijection_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_NatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefHypothesis_ suppressed *)
(* usable definition CONSTANT_FiniteNatInductiveDefConclusion_ suppressed *)
(* usable definition CONSTANT_IsTransitivelyClosedOn_ suppressed *)
(* usable definition CONSTANT_IsWellFoundedOn_ suppressed *)
(* usable definition CONSTANT_SetLessThan_ suppressed *)
(* usable definition CONSTANT_WFDefOn_ suppressed *)
(* usable definition CONSTANT_OpDefinesFcn_ suppressed *)
(* usable definition CONSTANT_WFInductiveDefines_ suppressed *)
(* usable definition CONSTANT_WFInductiveUnique_ suppressed *)
(* usable definition CONSTANT_TransitiveClosureOn_ suppressed *)
(* usable definition CONSTANT_OpToRel_ suppressed *)
(* usable definition CONSTANT_PreImage_ suppressed *)
(* usable definition CONSTANT_LexPairOrdering_ suppressed *)
(* usable definition CONSTANT_LexProductOrdering_ suppressed *)
(* usable definition CONSTANT_FiniteSubsetsOf_ suppressed *)
(* usable definition CONSTANT_StrictSubsetOrdering_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
(* usable definition CONSTANT_Edges_ suppressed *)
(* usable definition CONSTANT_NonLoopEdges_ suppressed *)
(* usable definition CONSTANT_SimpleGraphs_ suppressed *)
(* usable definition CONSTANT_Degree_ suppressed *)
fixes a_CONSTANTunde_Nodesunde_a
assumes v'155: "((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Nodesunde_a))))"
assumes v'156: "((greater (((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Nodesunde_a)))), ((Succ[0])))))"
fixes a_CONSTANTunde_Gunde_a
assumes a_CONSTANTunde_Gunde_a_in : "(a_CONSTANTunde_Gunde_a \<in> ((a_CONSTANTunde_SimpleGraphsunde_a ((a_CONSTANTunde_Nodesunde_a)))))"
fixes a_CONSTANTunde_nunde_a
assumes a_CONSTANTunde_nunde_a_in : "(a_CONSTANTunde_nunde_a \<in> (((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))))"
assumes v'182: "((a_CONSTANTunde_IsFiniteSetunde_a ((((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a_1. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) \\ ({(a_CONSTANTunde_nunde_a)}))))))"
assumes v'183: "((\<And> a_CONSTANTunde_Sunde_a :: c. (((a_CONSTANTunde_IsFiniteSetunde_a ((a_CONSTANTunde_Sunde_a)))) \<Longrightarrow> (\<And> a_CONSTANTunde_Opunde_a :: c => c. (((a_CONSTANTunde_IsFiniteSetunde_a ((setOfAll((a_CONSTANTunde_Sunde_a), %a_CONSTANTunde_xunde_a. ((a_CONSTANTunde_Opunde_a ((a_CONSTANTunde_xunde_a))))))))) & ((leq (((a_CONSTANTunde_Cardinalityunde_a ((setOfAll((a_CONSTANTunde_Sunde_a), %a_CONSTANTunde_xunde_a. ((a_CONSTANTunde_Opunde_a ((a_CONSTANTunde_xunde_a))))))))), ((a_CONSTANTunde_Cardinalityunde_a ((a_CONSTANTunde_Sunde_a))))))))))))"
shows "((a_CONSTANTunde_IsFiniteSetunde_a ((setOfAll((((((a_CONSTANTunde_Nodesunde_a) \\ (subsetOf((a_CONSTANTunde_Nodesunde_a), %a_CONSTANTunde_nunde_a_1. ((((a_CONSTANTunde_Degreeunde_a ((a_CONSTANTunde_nunde_a_1), (a_CONSTANTunde_Gunde_a)))) = ((0)))))))) \\ ({(a_CONSTANTunde_nunde_a)}))), %a_CONSTANTunde_munde_a. ({(a_CONSTANTunde_munde_a), (a_CONSTANTunde_nunde_a)}))))))"(is "PROP ?ob'93")
proof -
ML_command {* writeln "*** TLAPS ENTER 93"; *}
show "PROP ?ob'93"
using assms by auto
ML_command {* writeln "*** TLAPS EXIT 93"; *} qed
end

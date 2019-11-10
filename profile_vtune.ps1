Remove-Item -path .\resultdir -Recurse -Force -Confirm:$false -ErrorAction Ignore
Write-Host "Running Profiler."
amplxe-cl -collect hotspots -r resultdir -- target\debug\evtx_dump.exe .\samples\Application.evtx | Out-Null
Write-Host "Exporting to result.csv"
amplxe-cl -r resultdir -report top-down -report-output result.csv -format csv -csv-delimiter comma -call-stack-mode all -column="CPU Time:Self" -column="Module"  -filter "Function Stack" 
inferno-collapse-vtune result.csv > stacks.folded
cat stacks.folded | inferno-flamegraph > profile.svg
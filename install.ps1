#
# Doo Programming Language Installer for Windows
# One-line install: irm https://raw.githubusercontent.com/nynrathod/doolang/main/install.ps1 | iex
#

$ErrorActionPreference = "Stop"

# Installation directory
$InstallDir = "$env:USERPROFILE\.doo"
$BinDir = "$InstallDir\bin"

# GitHub repo info
$GithubRepo = "nynrathod/doolang"

function Write-Banner {
    Write-Host ""
    Write-Host "  ____              " -ForegroundColor Cyan
    Write-Host " |  _ \  ___   ___  " -ForegroundColor Cyan
    Write-Host " | | | |/ _ \ / _ \ " -ForegroundColor Cyan
    Write-Host " | |_| | (_) | (_) |" -ForegroundColor Cyan
    Write-Host " |____/ \___/ \___/ " -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Doo Programming Language Installer" -ForegroundColor White
    Write-Host ""
}

function Write-Info {
    param([string]$Message)
    Write-Host "[INFO] " -ForegroundColor Blue -NoNewline
    Write-Host $Message
}

function Write-Success {
    param([string]$Message)
    Write-Host "[SUCCESS] " -ForegroundColor Green -NoNewline
    Write-Host $Message
}

function Write-Warning-Custom {
    param([string]$Message)
    Write-Host "[WARN] " -ForegroundColor Yellow -NoNewline
    Write-Host $Message
}

function Write-Error-Custom {
    param([string]$Message)
    Write-Host "[ERROR] " -ForegroundColor Red -NoNewline
    Write-Host $Message
    exit 1
}

function Get-LatestVersion {
    Write-Info "Fetching latest version..."
    
    try {
        $releaseUrl = "https://api.github.com/repos/$GithubRepo/releases/latest"
        $response = Invoke-RestMethod -Uri $releaseUrl -Method Get -Headers @{ "User-Agent" = "Doo-Installer" }
        $script:Version = $response.tag_name
        $script:VersionNum = $Version -replace '^v', ''
        Write-Info "Latest version: $Version"
    }
    catch {
        Write-Error-Custom "Failed to fetch latest version. Check your internet connection. Error: $_"
    }
}

function Download-AndExtract {
    $downloadUrl = "https://github.com/$GithubRepo/releases/download/$Version/doo-windows-$VersionNum.zip"
    $tempDir = Join-Path $env:TEMP "doo-install-$(Get-Random)"
    $zipFile = Join-Path $tempDir "doo.zip"
    
    Write-Info "Downloading from: $downloadUrl"
    
    try {
        # Create temp directory
        New-Item -ItemType Directory -Path $tempDir -Force | Out-Null
        
        # Download
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri $downloadUrl -OutFile $zipFile -UseBasicParsing
        
        Write-Info "Extracting files..."
        
        # Create installation directory
        if (Test-Path $BinDir) {
            Remove-Item -Path $BinDir -Recurse -Force
        }
        New-Item -ItemType Directory -Path $BinDir -Force | Out-Null
        
        # Extract zip
        $extractDir = Join-Path $tempDir "extracted"
        Expand-Archive -Path $zipFile -DestinationPath $extractDir -Force
        
        # Find and copy all files to bin directory
        # Handle various folder structures: look for doo.exe in the extracted content
        $extractedItems = Get-ChildItem -Path $extractDir
        $dooExeFound = $false
        
        # Check if doo.exe is directly in extracted dir
        if (Test-Path (Join-Path $extractDir "doo.exe")) {
            Copy-Item -Path "$extractDir\*" -Destination $BinDir -Recurse -Force
            $dooExeFound = $true
        }
        else {
            # Look for doo.exe in subdirectories
            foreach ($item in $extractedItems) {
                if ($item.PSIsContainer) {
                    $possibleExe = Join-Path $item.FullName "doo.exe"
                    if (Test-Path $possibleExe) {
                        Copy-Item -Path "$($item.FullName)\*" -Destination $BinDir -Recurse -Force
                        $dooExeFound = $true
                        break
                    }
                }
            }
        }
        
        if (-not $dooExeFound) {
            # Fallback: just copy everything from first subdirectory
            $firstDir = $extractedItems | Where-Object { $_.PSIsContainer } | Select-Object -First 1
            if ($firstDir) {
                Copy-Item -Path "$($firstDir.FullName)\*" -Destination $BinDir -Recurse -Force
            }
            else {
                Copy-Item -Path "$extractDir\*" -Destination $BinDir -Recurse -Force
            }
        }
        
        # Cleanup
        Remove-Item -Path $tempDir -Recurse -Force
        
        Write-Success "Files extracted to $BinDir"
    }
    catch {
        Write-Error-Custom "Download or extraction failed. Error: $_"
    }
}

function Setup-Path {
    Write-Info "Setting up PATH..."
    
    $currentPath = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
    
    if ($currentPath -like "*$BinDir*") {
        Write-Info "PATH already contains $BinDir"
    }
    else {
        try {
            $newPath = $currentPath + ";" + $BinDir
            [Environment]::SetEnvironmentVariable("Path", $newPath, [EnvironmentVariableTarget]::User)
            Write-Success "Added $BinDir to user PATH"
        }
        catch {
            Write-Error-Custom "Failed to update PATH. Error: $_"
        }
    }
    
    # Update current session PATH
    $env:Path = $env:Path + ";" + $BinDir
}

function Check-Dependencies {
    Write-Info "Checking dependencies..."
    
    # Check for clang or MSVC
    $hasClang = $null -ne (Get-Command clang -ErrorAction SilentlyContinue)
    $hasMSVC = Test-Path "C:\Program Files\Microsoft Visual Studio"
    
    if (-not $hasClang -and -not $hasMSVC) {
        Write-Warning-Custom "No C compiler found. Doo may require clang or MSVC for linking."
        Write-Host ""
        Write-Host "  Install options:" -ForegroundColor White
        Write-Host "    1. LLVM/Clang: https://releases.llvm.org/download.html" -ForegroundColor Cyan
        Write-Host "    2. Visual Studio Build Tools: https://visualstudio.microsoft.com/visual-cpp-build-tools/" -ForegroundColor Cyan
        Write-Host ""
    }
    else {
        Write-Success "C compiler available"
    }
}

function Verify-Installation {
    Write-Info "Verifying installation..."
    
    $dooExe = Join-Path $BinDir "doo.exe"
    
    if (Test-Path $dooExe) {
        Write-Success "Doo installed successfully!"
        Write-Host ""
        Write-Host "Installation complete!" -ForegroundColor White
        Write-Host ""
        Write-Host "  Binary location: " -NoNewline
        Write-Host "$dooExe" -ForegroundColor Cyan
        Write-Host ""
        
        # Check if doo is accessible
        $dooCmd = Get-Command doo -ErrorAction SilentlyContinue
        if ($dooCmd) {
            Write-Host "  " -NoNewline
            Write-Host "doo" -ForegroundColor Green -NoNewline
            Write-Host " command is available in current session"
            Write-Host ""
            Write-Host "  Run " -NoNewline
            Write-Host "doo --help" -ForegroundColor Cyan -NoNewline
            Write-Host " to get started"
        }
        else {
            Write-Host "  " -NoNewline
            Write-Host "!" -ForegroundColor Yellow -NoNewline
            Write-Host " PATH has been updated. To use doo in this terminal:"
            Write-Host ""
            Write-Host "    1. Close and reopen this terminal, OR" -ForegroundColor Yellow
            Write-Host "    2. Run: " -NoNewline -ForegroundColor Yellow
            Write-Host '$env:Path = [Environment]::GetEnvironmentVariable("Path", "User")' -ForegroundColor Cyan
            Write-Host ""
        }
        Write-Host ""
    }
    else {
        Write-Error-Custom "Installation verification failed. Binary not found at $dooExe"
    }
}

function Refresh-Environment {
    Write-Info "Refreshing environment variables..."
    
    # Refresh PATH for current session
    $env:Path = [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::Machine) + ";" + [Environment]::GetEnvironmentVariable("Path", [EnvironmentVariableTarget]::User)
}

# Main installation flow
function Main {
    Write-Banner
    Get-LatestVersion
    Download-AndExtract
    Setup-Path
    Refresh-Environment
    Check-Dependencies
    Verify-Installation
}

Main

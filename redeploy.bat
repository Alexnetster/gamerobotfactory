@echo off
setlocal

echo ===================================================
echo  gamerobotfactory - local Docker redeploy
echo ===================================================

echo.
echo [1/3] Pulling latest changes from git...
git pull
if errorlevel 1 (
    echo.
    echo git pull failed - resolve this manually before continuing.
    echo (Uncommitted local changes or a merge conflict is the usual cause.)
    pause
    exit /b 1
)

echo.
echo [2/3] Rebuilding the Docker image and restarting the container...
docker compose up --build -d --force-recreate
if errorlevel 1 (
    echo.
    echo docker compose failed - is Docker Desktop running?
    pause
    exit /b 1
)

echo.
echo [3/3] Done. Server should be live at http://localhost:8081
echo (8080 is occupied by an unrelated container on this machine - override with the HOST_PORT env var if 8081 also conflicts)
echo (docker compose logs -f   to watch logs, docker compose down   to stop)
echo.
pause

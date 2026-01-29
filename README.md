# RepoRocket 🚀

(OLD PYTHON)

**RepoRocket** is a cross-platform application manager that allows users to **install, update, and organize software directly from GitHub, GitLab, and other Git-based repositories**. Designed for flexibility, it simplifies managing open-source applications while offering customization options for users.

## 🔹 Features:
- **Repository-Based Installation** – Download and update applications directly from Git sources.
- **Multi-Release Support** – Choose between stable, beta, and nightly builds.
- **App Management** – Launch, rename, and customize applications with custom artwork.
- **Flatpak & AppImage Integration** – Install Linux applications with ease.
## Setup

### Prerequisites
- Python 3.6 or higher
- Pip (Python package installer)

### Installation
1. **Clone the Repository**:
    ```bash
    git clone https://github.com/yourusername/reporocket.git
    cd reporocket
    ```

2. **Install Dependencies**:
    ```bash
    pip install -r requirements.txt
    ```

### Running the Application
To run the application, execute the following command:
```bash
python RepoRocket.py
```

## Building the Application

### Windows
1. **Navigate to the Scripts Directory**:
    ```bash
    cd scripts
    ```

2. **Run the Build Script**:
    ```bat
    build_windows.bat
    ```

### Linux
1. **Navigate to the Scripts Directory**:
    ```bash
    cd scripts
    ```

2. **Run the Build Script**:
    ```bash
    ./build_linux.sh
    ```

## Directory Structure
- `applications/`: Stores downloaded GitHub applications.
- `saves/`: Stores save files for applications, synced with Steam Cloud.
- `img/`: Images for the launcher UI.
- `scripts/`: Build scripts for Windows and Linux.

## Contributing
Contributions are welcome! Please fork the repository and submit a pull request with your changes.

## License
This project is licensed under the MIT License. See the [LICENSE](LICENSE) file for details.

![logo](https://github.com/user-attachments/assets/e933c40b-98d0-41d7-bba3-cffcfa00da33)

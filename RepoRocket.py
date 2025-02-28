import sys
import os
import requests
import zipfile
import json
import pygame
from PyQt6.QtWidgets import (
    QApplication, QMainWindow, QVBoxLayout, QHBoxLayout, QPushButton,
    QLabel, QLineEdit, QStackedWidget, QWidget, QScrollArea, QComboBox,
    QGridLayout, QMenu, QFileDialog, QCheckBox, QProgressBar, QDialog, QListWidget, QListWidgetItem, QSplitter, QMessageBox
)
from PyQt6.QtCore import Qt, QTimer, QPropertyAnimation, QEasingCurve, QEvent, QPoint, QUrl
from PyQt6.QtGui import QPixmap, QIcon, QAction, QPainter, QBrush, QFontDatabase, QKeyEvent
from PyQt6.QtWebEngineWidgets import QWebEngineView
import shutil
import traceback
from steamgrid import SteamGridDB, StyleType, MimeType, ImageType
import qdarkstyle
import yaml
import importlib.util

# Initialize SteamGridDB with your API key
sgdb = SteamGridDB('40f20195948fb2489554d4c9e5ee8ef9')

class RepoRocket(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("RepoRocket")
        self.setGeometry(100, 100, 1280, 720)
        self.setWindowIcon(QIcon("img/logo.png"))  # Set the application icon
        self.settings_path = os.path.join("saves", "reporocket", "settings.json")
        self.config_path = os.path.join("saves", "reporocket", "config.json")
        self.error_log_path = os.path.join("saves", "reporocket", "errorlogs.json")
        self.themes_path = os.path.join("themes")
        self.plugins_path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "plugins")
        self.create_folder_structure()
        self.init_ui()
        self.status_bar = self.statusBar()
        self.progress_bar = QProgressBar()
        self.status_bar.addPermanentWidget(self.progress_bar)
        self.progress_bar.setVisible(False)
        self.load_config()
        self.settings = {}
        self.load_settings()
        self.load_plugins()

        # Initialize pygame and gamepad in a separate thread to avoid blocking the UI
        QTimer.singleShot(0, self.init_gamepad_async)

        self.current_focus = None

    def create_folder_structure(self):
        os.makedirs("applications", exist_ok=True)
        os.makedirs("saves/reporocket", exist_ok=True)
        os.makedirs(self.themes_path, exist_ok=True)
        os.makedirs(self.plugins_path, exist_ok=True)
        if not os.path.exists(self.error_log_path):
            with open(self.error_log_path, 'w') as f:
                json.dump([], f)

    def init_ui(self):
        # Apply default dark theme
        self.setStyleSheet(qdarkstyle.load_stylesheet(qt_api='pyqt6'))

        # Main layout
        main_layout = QHBoxLayout()

        # Sidebar
        self.side_panel = QVBoxLayout()
        self.side_panel.setAlignment(Qt.AlignmentFlag.AlignTop)

        self.side_panel_widget = QWidget()
        self.side_panel_widget.setLayout(self.side_panel)
        self.side_panel_widget.setFixedWidth(200)
        self.side_panel_widget.setStyleSheet("background-color: #0d0d0d;")

        # Add buttons to the sidebar
        self.add_button("Search", self.show_search_page)
        self.add_button("Library", self.show_library_page)
        self.add_button("Settings", self.show_settings_page)

        # Main content area
        self.main_content = QStackedWidget(self)
        self.search_page = self.create_search_page()
        self.repo_detail_page = self.create_repo_detail_page()
        self.library_page = self.create_library_page()
        self.settings_page = self.create_settings_page()
        self.main_content.addWidget(self.search_page)
        self.main_content.addWidget(self.repo_detail_page)
        self.main_content.addWidget(self.library_page)
        self.main_content.addWidget(self.settings_page)

        # Add widgets to layout
        main_layout.addWidget(self.side_panel_widget)
        main_layout.addWidget(self.main_content, 1)

        # Set central widget
        central_widget = QWidget()
        central_widget.setLayout(main_layout)
        self.setCentralWidget(central_widget)

        # Load config
        self.config_path = os.path.join("saves", "reporocket", "config.json")
        self.load_config()

        # Set initial focus
        self.current_focus = self.side_panel.itemAt(0).widget()
        self.current_focus.setFocus()

        self.setFocusPolicy(Qt.FocusPolicy.StrongFocus)  # Ensure the main window can receive focus
        self.installEventFilter(self)  # Install event filter to capture key events

        self.fullscreen_selector.setCurrentText("Windowed" if not self.isFullScreen() else "Fullscreen")
        self.load_settings()

    def add_button(self, label, callback):
        # Button with text only
        btn = QPushButton(label)
        btn.setFixedSize(180, 50)
        btn.setStyleSheet("""QPushButton {
            background-color: transparent;
            color: white;
            border: none;
            text-align: left;
            padding-left: 10px;
            font-size: 18px;
            font-family: Arial;
        }
        QPushButton:hover {
            background-color: #2e2e2e;
        }
        QPushButton:focus {
            background-color: #3e3e3e;
        }""")
        btn.clicked.connect(callback)
        self.side_panel.addWidget(btn)

    def init_gamepad_async(self):
        # Initialize pygame and gamepad
        pygame.init()
        self.gamepads = []
        self.init_gamepads()
        self.timer = QTimer(self)
        self.timer.timeout.connect(self.poll_gamepads)
        self.timer.start(100)

    def create_search_page(self):
        page = QWidget()
        layout = QVBoxLayout()

        search_bar = QLineEdit()
        search_bar.setPlaceholderText("Search repositories...")
        search_bar.setStyleSheet("font-size: 18px; padding: 10px; font-family: Arial;")
        search_bar.returnPressed.connect(lambda: self.perform_search(search_bar.text()))
        layout.addWidget(search_bar)

        self.repo_selector = QComboBox()
        self.repo_selector.addItems(["GitHub", "GitLab", "Internet Archive"])
        self.repo_selector.setStyleSheet("font-size: 18px; padding: 10px; font-family: Arial;")
        self.repo_selector.currentIndexChanged.connect(lambda: self.perform_search(search_bar.text()))
        layout.addWidget(self.repo_selector)

        self.results_area = QScrollArea()
        self.results_area.setWidgetResizable(True)
        self.results_widget = QWidget()
        self.results_layout = QVBoxLayout()
        self.results_widget.setLayout(self.results_layout)
        self.results_area.setWidget(self.results_widget)
        layout.addWidget(self.results_area)

        page.setLayout(layout)
        return page

    def perform_search(self, query):
        # Clear previous results
        for i in reversed(range(self.results_layout.count())):
            self.results_layout.itemAt(i).widget().deleteLater()

        # Fetch search results based on selected repository
        try:
            if self.repo_selector.currentText() == "GitHub":
                api_url = f"https://api.github.com/search/repositories?q={query}"
            elif self.repo_selector.currentText() == "GitLab":
                api_url = f"https://gitlab.com/api/v4/projects?search={query}"
            elif self.repo_selector.currentText() == "Internet Archive":
                api_url = f"https://archive.org/advancedsearch.php?q={query}&fl[]=identifier,title,creator&output=json"
            else:
                raise Exception("Unsupported repository")

            response = requests.get(api_url)
            if response.status_code == 200:
                if self.repo_selector.currentText() == "GitHub":
                    repos = response.json().get("items", [])
                elif self.repo_selector.currentText() == "GitLab":
                    repos = response.json()
                elif self.repo_selector.currentText() == "Internet Archive":
                    repos = response.json().get("response", {}).get("docs", [])
                else:
                    repos = []

                for repo in repos[:10]:  # Limit to top 10 results
                    if self.repo_selector.currentText() == "GitHub":
                        repo_name = repo['name']
                        owner_name = repo['owner']['login']
                    elif self.repo_selector.currentText() == "GitLab":
                        repo_name = repo['name']
                        owner_name = repo['namespace']['name']
                    elif self.repo_selector.currentText() == "Internet Archive":
                        repo_name = repo['title']
                        owner_name = repo['creator']

                    button = QPushButton(f"{repo_name} by {owner_name}")
                    button.setStyleSheet("""QPushButton {
                        background-color: #2e2e2e;
                        color: white;
                        padding: 10px;
                        text-align: left;
                        font-size: 16px;
                        font-family: Arial;
                    }
                    QPushButton:hover {
                        background-color: #3e3e3e;
                    }
                    QPushButton:focus {
                        background-color: #3e3e3e;
                    }""")
                    button.clicked.connect(lambda _, r=repo: self.show_repo_details(r))
                    self.results_layout.addWidget(button)
            else:
                raise Exception("API error")
        except Exception as e:
            error_message = f"Error fetching results: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)
            error_label = QLabel(f"Error fetching results: {e}")
            error_label.setStyleSheet("color: red; font-size: 16px; font-family: Arial;")
            self.results_layout.addWidget(error_label)

    def create_repo_detail_page(self):
        page = QWidget()
        layout = QVBoxLayout()

        self.repo_title = QLabel("")
        self.repo_title.setStyleSheet("font-size: 24px; font-family: Arial; color: white;")
        layout.addWidget(self.repo_title)

        self.repo_description = QLabel("")
        self.repo_description.setWordWrap(True)
        self.repo_description.setStyleSheet("font-size: 16px; font-family: Arial; color: white;")
        layout.addWidget(self.repo_description)

        self.github_actions_toggle = QCheckBox("Enable GitHub Actions")
        self.github_actions_toggle.setStyleSheet("font-size: 16px; font-family: Arial; color: white;")
        self.github_actions_toggle.stateChanged.connect(self.toggle_github_actions)
        layout.addWidget(self.github_actions_toggle)

        self.release_selector = QComboBox()
        self.release_selector.setStyleSheet("font-size: 16px; font-family: Arial; padding: 5px;")
        self.release_selector.currentIndexChanged.connect(self.populate_files)
        layout.addWidget(self.release_selector)

        self.file_selector = QComboBox()
        self.file_selector.setStyleSheet("font-size: 16px; font-family: Arial; padding: 5px;")
        layout.addWidget(self.file_selector)

        # Add buttons
        button_layout = QHBoxLayout()
        download_button = QPushButton("Download")
        download_button.setStyleSheet("""QPushButton {
            background-color: #2e2e2e;
            color: white;
            padding: 10px;
            font-size: 16px;
            font-family: Arial;
        }
        QPushButton:hover {
            background-color: #3e3e3e;
        }
        QPushButton:focus {
            background-color: #3e3e3e;
        }""")
        download_button.clicked.connect(self.download_selected_file)
        button_layout.addWidget(download_button)

        back_button = QPushButton("Back to Search")
        back_button.setStyleSheet("""QPushButton {
            background-color: #2e2e2e;
            color: white;
            padding: 10px;
            font-size: 16px;
            font-family: Arial;
        }
        QPushButton:hover {
            background-color: #3e3e3e;
        }
        QPushButton:focus {
            background-color: #3e3e3e;
        }""")
        back_button.clicked.connect(self.show_search_page)
        button_layout.addWidget(back_button)

        layout.addLayout(button_layout)
        page.setLayout(layout)
        return page

    def toggle_github_actions(self, state):
        self.settings["github_actions_enabled"] = state == Qt.CheckState.Checked
        self.save_settings()
        self.show_repo_details(self.current_repo)

    def fetch_github_actions(self):
        self.release_selector.clear()
        try:
            owner = self.current_repo['owner']['login']
            repo_name = self.current_repo['name']
            api_url = f"https://api.github.com/repos/{owner}/{repo_name}/actions/workflows"
            response = requests.get(api_url)
            if response.status_code == 200:
                workflows = response.json().get("workflows", [])
                for workflow in workflows:
                    self.release_selector.addItem(workflow['name'], workflow['id'])
                if not workflows:
                    self.release_selector.addItem("No workflows available", None)
            else:
                self.release_selector.addItem("Error fetching workflows", None)
        except Exception as e:
            self.release_selector.addItem(f"Error: {e}", None)

    def fetch_releases(self):
        self.release_selector.clear()
          try:
            owner = self.current_repo['owner']['login']
            repo_name = self.current_repo['name']
            api_url = f"https://api.github.com/repos/{owner}/{repo_name}/releases"
            response = requests.get(api_url)
            if response.status_code == 200:
                releases = response.json()
                for release in releases:
                    self.release_selector.addItem(release['tag_name'], release['assets'])
                if not releases:
                    self.release_selector.addItem("No releases available", None)
            else:
                self.release_selector.addItem("Error fetching releases", None)
        except Exception as e:
            self.release_selector.addItem(f"Error: {e}", None)

    def populate_files(self):
        self.file_selector.clear()
        if self.github_actions_toggle.isChecked():
            self.populate_github_action_artifacts()
        else:
            self.populate_release_files()

    def populate_github_action_artifacts(self):
        workflow_id = self.release_selector.currentData()
        if workflow_id:
            try:
                owner = self.current_repo['owner']['login']
                repo_name = self.current_repo['name']
                api_url = f"https://api.github.com/repos/{owner}/{repo_name}/actions/runs?workflow_id={workflow_id}"
                response = requests.get(api_url)
                if response.status_code == 200:
                    runs = response.json().get("workflow_runs", [])
                    if runs:
                        latest_run_id = runs[0]['id']
                        artifacts_url = f"https://api.github.com/repos/{owner}/{repo_name}/actions/runs/{latest_run_id}/artifacts"
                        artifacts_response = requests.get(artifacts_url)
                        if artifacts_response.status_code == 200:
                            artifacts = artifacts_response.json().get("artifacts", [])
                            for artifact in artifacts:
                                self.file_selector.addItem(artifact['name'], artifact['archive_download_url'])
                            if not artifacts:
                                self.file_selector.addItem("No artifacts available", None)
                        else:
                            self.file_selector.addItem("Error fetching artifacts", None)
                    else:
                        self.file_selector.addItem("No workflow runs available", None)
                else:
                    self.file_selector.addItem("Error fetching workflow runs", None)
            except Exception as e:
                self.file_selector.addItem(f"Error: {e}", None)

    def populate_release_files(self):
        assets = self.release_selector.currentData()
        if assets:
            for asset in assets:
                self.file_selector.addItem(asset['name'], asset['browser_download_url'])

    def show_repo_details(self, repo):
        self.current_repo = repo
        if self.repo_selector.currentText() == "GitHub":
            owner = repo['owner']['login']
            repo_name = repo['name']
        elif self.repo_selector.currentText() == "GitLab":
            owner = repo['namespace']['name']
            repo_name = repo['name']
        elif self.repo_selector.currentText() == "Internet Archive":
            owner = repo['creator']
            repo_name = repo['title']

        self.repo_title.setText(repo_name)
        self.repo_description.setText(repo.get('description', 'No description available.'))

        if self.settings.get("github_actions_enabled", False):
            self.github_actions_toggle.setChecked(True)
            self.fetch_github_actions()
        else:
            self.github_actions_toggle.setChecked(False)
            self.fetch_releases()

        self.main_content.setCurrentWidget(self.repo_detail_page)

    def download_selected_file(self):
        selected_url = self.file_selector.currentData()
        if not selected_url:
            self.repo_description.setText("No valid file selected.")
            return

        try:
            repo_name = self.current_repo['name'] if self.repo_selector.currentText() != "Internet Archive" else self.current_repo['title']
            self.download_file(selected_url, repo_name)
        except Exception as e:
            error_message = f"Error during download: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)
            self.repo_description.setText(f"Error during download: {e}")
            self.progress_bar.setVisible(False)

    def download_file(self, url, repo_name):
        response = requests.get(url, stream=True)
        file_name = url.split("/")[-1]
        app_folder = os.path.join("applications", repo_name)
        os.makedirs(app_folder, exist_ok=True)
        save_path = os.path.join(app_folder, file_name)
        
        total_size = int(response.headers.get('content-length', 0))
        self.progress_bar.setMaximum(total_size)
        self.progress_bar.setValue(0)
        self.progress_bar.setVisible(True)

        with open(save_path, "wb") as f:
            for chunk in response.iter_content(chunk_size=1024):
                if chunk:
                    f.write(chunk)
                    self.progress_bar.setValue(self.progress_bar.value() + len(chunk))

        self.progress_bar.setVisible(False)

        if zipfile.is_zipfile(save_path):
            self.repo_description.setText("Extracting files...")
            self.progress_bar.setVisible(True)
            self.unzip_and_clean(save_path, repo_name)
            self.progress_bar.setVisible(False)
            self.repo_description.setText(f"Downloaded and extracted to {app_folder}")
            self.prompt_for_executable(repo_name)
        elif file_name.endswith(".html"):
            self.repo_description.setText(f"Downloaded HTML file to {app_folder}")
            self.display_html_content(save_path)
        else:
            self.repo_description.setText(f"Downloaded to {app_folder}")
            self.prompt_for_executable(repo_name)

    def display_html_content(self, html_path):
        self.html_viewer = QWebEngineView()
        self.html_viewer.setUrl(QUrl.fromLocalFile(html_path))
        self.main_content.addWidget(self.html_viewer)
        self.main_content.setCurrentWidget(self.html_viewer)

    def unzip_and_clean(self, zip_path, repo_name):
        extract_path = os.path.join("applications", repo_name)
        os.makedirs(extract_path, exist_ok=True)
        with zipfile.ZipFile(zip_path, "r") as zip_ref:
            total_files = len(zip_ref.infolist())
            self.progress_bar.setMaximum(total_files)
            self.progress_bar.setValue(0)
            for i, file in enumerate(zip_ref.infolist()):
                zip_ref.extract(file, extract_path)
                self.progress_bar.setValue(i + 1)
        os.remove(zip_path)

        # Ensure no nested folders
        for root, dirs, files in os.walk(extract_path):
            if len(dirs) == 1 and not files:
                nested_folder = os.path.join(root, dirs[0])
                for item in os.listdir(nested_folder):
                    shutil.move(os.path.join(nested_folder, item), os.path.join(extract_path, item))
                os.rmdir(nested_folder)
                break

    def prompt_for_executable(self, repo_name):
        self.executable_selector = QWidget()
        layout = QVBoxLayout()

        label = QLabel("Select Executable")
        label.setStyleSheet("font-size: 18px; font-family: Arial; color: white;")
        layout.addWidget(label)

        list_widget = QListWidget()
        list_widget.setStyleSheet("""
            QListWidget {
                font-size: 16px;
                font-family: Arial;
                padding: 10px;
                background-color: #2e2e2e;
                color: white;
            }
            QListWidget::item {
                padding: 10px;
                border: 1px solid #3e3e3e;
                margin-bottom: 5px;
            }
            QListWidget::item:selected {
                background-color: #3e3e3e;
                border: 1px solid #5e5e5e;
            }
        """)
        app_folder = os.path.join("applications", repo_name)
        for root, dirs, files in os.walk(app_folder):
            for file in files:
                if file.endswith((".exe", ".bat", ".sh", ".appimage", ".app")):
                    item = QListWidgetItem(file)
                    item.setData(Qt.ItemDataRole.UserRole, os.path.join(root, file))
                    list_widget.addItem(item)

        list_widget.itemDoubleClicked.connect(lambda item: self.set_executable(repo_name, item.data(Qt.ItemDataRole.UserRole)))
        layout.addWidget(list_widget)

        self.executable_selector.setLayout(layout)
        self.main_content.addWidget(self.executable_selector)
        self.main_content.setCurrentWidget(self.executable_selector)

    def set_executable(self, repo_name, executable_path):
        self.config[repo_name] = {"executable": executable_path}
        self.save_config()
        self.main_content.setCurrentWidget(self.search_page)

    def show_search_page(self):
        self.main_content.setCurrentWidget(self.search_page)

    def show_library_page(self):
        self.main_content.setCurrentWidget(self.library_page)
        self.update_library_page()

    def show_settings_page(self):
        self.main_content.setCurrentWidget(self.settings_page)

    def create_library_page(self):
        page = QWidget()
        layout = QVBoxLayout()

        self.library_scroll_area = QScrollArea()
        self.library_scroll_area.setWidgetResizable(True)
        self.library_widget = QWidget()
        self.library_layout = QGridLayout()
        self.library_widget.setLayout(self.library_layout)
        self.library_scroll_area.setWidget(self.library_widget)
        layout.addWidget(self.library_scroll_area)

        page.setLayout(layout)
        self.main_content.addWidget(page)
        self.resizeEvent = self.update_library_page_on_resize
        return page

    def update_library_page_on_resize(self, event):
        self.update_library_page()
        super().resizeEvent(event)

    def update_library_page(self):
        # Clear previous library items
        for i in reversed(range(self.library_layout.count())):
            self.library_layout.itemAt(i).widget().deleteLater()

        # Calculate the number of columns based on the window width
        window_width = self.library_scroll_area.width()
        tile_width = 430  # Width of each tile
        num_columns = max(1, window_width // tile_width)

        # List downloaded applications
        apps_dir = "applications"
        if os.path.exists(apps_dir) and os.listdir(apps_dir):
            row, col = 0, 0
            for app_name in os.listdir(apps_dir):
                app_path = os.path.join(apps_dir, app_name)
                if os.path.isdir(app_path):
                    tile = self.create_app_tile(app_name)
                    self.library_layout.addWidget(tile, row, col)
                    col += 1
                    if col >= num_columns:
                        col = 0
                        row += 1
        else:
            empty_widget = QWidget()
            empty_layout = QVBoxLayout()
            empty_widget.setLayout(empty_layout)
            empty_widget.setStyleSheet("""
                QWidget {
                    background-color: #2e2e2e;
                    border-radius: 20px;
                    padding: 20px;
                }
            """)

            empty_label = QLabel("Library is Empty. Download something from the Search tab.")
            empty_label.setStyleSheet("font-size: 18px; font-family: Arial; color: white;")
            empty_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            empty_layout.addWidget(empty_label)

            self.library_layout.addWidget(empty_widget, 0, 0, 1, num_columns, Qt.AlignmentFlag.AlignCenter)

    def create_app_tile(self, app_name):
        button = QPushButton()
        button.setFixedSize(430, 200)  # Aspect ratio 2.14:1
        button.setStyleSheet("""
            QPushButton {
                border-radius: 20px;
                background-color: #2e2e2e;
                color: white;
                font-size: 16px;
                font-family: Arial;
                text-align: center;
            }
            QPushButton:focus {
                background-color: #3e3e3e;
                border: 2px solid white;  # Add white border when focused
            }
        """)

        # Load artwork if available
        artwork_path = os.path.join("saves", "reporocket", "artwork", f"{app_name}.png")
        if os.path.exists(artwork_path):
            pixmap = QPixmap(artwork_path).scaled(button.size(), Qt.AspectRatioMode.KeepAspectRatioByExpanding, Qt.TransformationMode.SmoothTransformation)
            mask = QPixmap(button.size())
            mask.fill(Qt.GlobalColor.transparent)
            painter = QPainter(mask)
            painter.setRenderHint(QPainter.RenderHint.Antialiasing)
            painter.setBrush(QBrush(pixmap))
            painter.drawRoundedRect(0, 0, button.width(), button.height(), 20, 20)
            painter.end()
            button.setIcon(QIcon(mask))
            button.setIconSize(button.size())
        else:
            button.setText(app_name)

        button.clicked.connect(lambda: self.launch_app(app_name))
        button.setContextMenuPolicy(Qt.ContextMenuPolicy.CustomContextMenu)
        button.customContextMenuRequested.connect(lambda pos: self.show_context_menu(pos, app_name, button))

        return button

    def change_artwork(self, app_name, button):
        self.current_app_name = app_name
        self.artwork_button = button
        self.show_artwork_search_page()

    def show_artwork_search_page(self):
        self.artwork_search_page = QWidget()
        layout = QVBoxLayout()

        search_bar = QLineEdit()
        search_bar.setPlaceholderText("Search for games on SteamGridDB...")
        search_bar.setStyleSheet("font-size: 18px; padding: 10px; font-family: Arial;")
        search_bar.returnPressed.connect(lambda: self.perform_artwork_search(search_bar.text()))
        layout.addWidget(search_bar)

        self.artwork_results_area = QScrollArea()
        self.artwork_results_area.setWidgetResizable(True)
        self.artwork_results_widget = QWidget()
        self.artwork_results_layout = QVBoxLayout()
        self.artwork_results_widget.setLayout(self.artwork_results_layout)
        self.artwork_results_area.setWidget(self.artwork_results_widget)
        layout.addWidget(self.artwork_results_area)

        self.artwork_search_page.setLayout(layout)
        self.main_content.addWidget(self.artwork_search_page)
        self.main_content.setCurrentWidget(self.artwork_search_page)

    def perform_artwork_search(self, query):
        # Clear previous results
        for i in reversed(range(self.artwork_results_layout.count())):
            self.artwork_results_layout.itemAt(i).widget().deleteLater()

        # Search for games on SteamGridDB
        try:
            result = sgdb.search_game(query)
            if not result:
                raise Exception("No games found on SteamGridDB")

            for game in result:
                game_name = game.name
                button = QPushButton(game_name)
                button.setStyleSheet("""QPushButton {
                    background-color: #2e2e2e;
                    color: white;
                    padding: 10px;
                    text-align: left;
                    font-size: 16px;
                    font-family: Arial;
                }
                QPushButton:hover {
                    background-color: #3e3e3e;
                }""")
                button.clicked.connect(lambda _, g=game: self.show_artwork_selection_page(g))
                self.artwork_results_layout.addWidget(button)
        except Exception as e:
            error_message = f"Error searching for games: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)
            error_label = QLabel(f"Error searching for games: {e}")
            error_label.setStyleSheet("color: red; font-size: 16px; font-family: Arial;")
            self.artwork_results_layout.addWidget(error_label)

    def show_artwork_selection_page(self, game):
        self.artwork_selection_page = QWidget()
        layout = QVBoxLayout()

        self.artwork_selection_area = QScrollArea()
        self.artwork_selection_area.setWidgetResizable(True)
        self.artwork_selection_widget = QWidget()
        self.artwork_selection_layout = QGridLayout()
        self.artwork_selection_widget.setLayout(self.artwork_selection_layout)
        self.artwork_selection_area.setWidget(self.artwork_selection_widget)
        layout.addWidget(self.artwork_selection_area)

        self.pagination_layout = QHBoxLayout()
        self.prev_button = QPushButton("Previous")
        self.prev_button.clicked.connect(lambda: self.load_artwork_page(game, self.current_page - 1))
        self.next_button = QPushButton("Next")
        self.next_button.clicked.connect(lambda: self.load_artwork_page(game, self.current_page + 1))
        self.pagination_layout.addWidget(self.prev_button)
        self.pagination_layout.addWidget(self.next_button)
        layout.addLayout(self.pagination_layout)

        self.artwork_selection_page.setLayout(layout)
        self.main_content.addWidget(self.artwork_selection_page)
        self.main_content.setCurrentWidget(self.artwork_selection_page)

        self.current_page = 0
        self.load_artwork_page(game, self.current_page)

    def load_artwork_page(self, game, page):
        self.current_page = page
        for i in reversed(range(self.artwork_selection_layout.count())):
            self.artwork_selection_layout.itemAt(i).widget().deleteLater()

        try:
            grids = sgdb.get_grids_by_gameid(game_ids=[game.id])
            if not grids:
                raise Exception("No suitable artwork found")

            start_index = page * 12
            end_index = start_index + 12
            grids_page = [grid for grid in grids if grid.width > grid.height][start_index:end_index]

            for i, grid in enumerate(grids_page):
                grid_url = grid.url
                button = QPushButton()
                button.setFixedSize(460, 215)  # Adjusted size for better display
                loading_icon = QPixmap("path/to/loading_icon.png")  # Replace with actual path to loading icon
                button.setIcon(QIcon(loading_icon))
                button.setIconSize(button.size())
                self.artwork_selection_layout.addWidget(button, i // 6, i % 6)

                def load_image(button, url):
                    pixmap = QPixmap()
                    pixmap.loadFromData(requests.get(url).content)
                    button.setIcon(QIcon(pixmap))
                    button.setIconSize(button.size())
                    button.clicked.connect(lambda _, url=url: self.download_and_apply_artwork(url))

                QTimer.singleShot(0, lambda b=button, u=grid_url: load_image(b, u))

            self.prev_button.setEnabled(page > 0)
            self.next_button.setEnabled(end_index < len(grids_page))
        except Exception as e:
            error_message = f"Error fetching artwork: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)
            error_label = QLabel(f"Error fetching artwork: {e}")
            error_label.setStyleSheet("color: red; font-size: 16px; font-family: Arial;")
            self.artwork_selection_layout.addWidget(error_label)

    def download_and_apply_artwork(self, url):
        try:
            response = requests.get(url, stream=True)
            if response.status_code == 200:
                artwork_dir = os.path.join("saves", "reporocket", "artwork")
                os.makedirs(artwork_dir, exist_ok=True)
                artwork_path = os.path.join(artwork_dir, f"{self.current_app_name}.png")
                with open(artwork_path, 'wb') as f:
                    for chunk in response.iter_content(1024):
                        f.write(chunk)

                # Update the button icon
                pixmap = QPixmap(artwork_path).scaled(self.artwork_button.size(), Qt.AspectRatioMode.KeepAspectRatioByExpanding, Qt.TransformationMode.SmoothTransformation)
                mask = QPixmap(self.artwork_button.size())
                mask.fill(Qt.GlobalColor.transparent)
                painter = QPainter(mask)
                painter.setRenderHint(QPainter.RenderHint.Antialiasing)
                painter.setBrush(QBrush(pixmap))
                painter.drawRoundedRect(0, 0, self.artwork_button.width(), self.artwork_button.height(), 20, 20)
                painter.end()
                self.artwork_button.setIcon(QIcon(mask))
                self.artwork_button.setIconSize(self.artwork_button.size())

                # Go back to the library page
                self.show_library_page()
            else:
                raise Exception("Failed to download artwork")
        except Exception as e:
            error_message = f"Error downloading artwork: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)
            print(error_message)

    def launch_app(self, app_name):
        executable_path = self.config.get(app_name, {}).get("executable")
        if executable_path and os.path.exists(executable_path):
            try:
                os.startfile(executable_path)
            except OSError as e:
                error_message = f"Error launching app: {e}\n{traceback.format_exc()}"
                self.log_error(error_message)
                if e.errno == 5:  # Access is denied
                    self.prompt_for_executable(app_name)
                else:
                    raise

    def create_settings_page(self):
        page = QWidget()
        layout = QVBoxLayout()

        settings_label = QLabel("Settings")
        settings_label.setStyleSheet("font-size: 24px; font-family: Arial; color: white;")
        layout.addWidget(settings_label)

        # Theme selection
        theme_label = QLabel("Theme")
        theme_label.setStyleSheet("font-size: 18px; font-family: Arial; color: white;")
        layout.addWidget(theme_label)

        self.theme_selector = QComboBox()
        self.load_themes()
        self.theme_selector.setStyleSheet("font-size: 18px; font-family: Arial; padding: 10px;")
        self.theme_selector.currentIndexChanged.connect(self.change_theme)
        layout.addWidget(self.theme_selector)

        # Fullscreen toggle
        self.fullscreen_selector = QComboBox()
        self.fullscreen_selector.addItems(["Windowed", "Fullscreen"])
        self.fullscreen_selector.setStyleSheet("font-size: 18px; font-family: Arial; color: white;")
        self.fullscreen_selector.setCurrentText("Windowed" if not self.isFullScreen() else "Fullscreen")
        self.fullscreen_selector.currentIndexChanged.connect(self.toggle_fullscreen)
        layout.addWidget(self.fullscreen_selector)

        import_rrct_button = QPushButton("Import RRCT")
        import_rrct_button.setStyleSheet("""
            QPushButton {
                font-size: 18px; font-family: Arial; color: white; background-color: #2e2e2e;
            }
            QPushButton:hover {
                background-color: #3e3e3e;
            }
        """)
        import_rrct_button.clicked.connect(self.import_rrct)
        layout.addWidget(import_rrct_button)

        test_error_button = QPushButton("Test Error Dump")
        test_error_button.setStyleSheet("""
            QPushButton {
                font-size: 18px; font-family: Arial; color: white; background-color: #2e2e2e;
            }
            QPushButton:hover {
                background-color: #3e3e3e;
            }
        """)
        test_error_button.clicked.connect(self.test_error_dump)
        layout.addWidget(test_error_button)

        layout.addStretch()
        page.setLayout(layout)
        return page

    def toggle_fullscreen(self, index):
        if self.fullscreen_selector.currentText() == "Fullscreen":
            self.showFullScreen()
        else:
            self.showNormal()
        self.save_settings()

    def import_rrct(self):
        try:
            file_path, _ = QFileDialog.getOpenFileName(self, "Import RRCT", "", "RepoRocket Custom Theme (*.rrct)")
            if file_path:
                theme_name = os.path.splitext(os.path.basename(file_path))[0]
                theme_folder = os.path.join(self.themes_path, theme_name)
                os.makedirs(theme_folder, exist_ok=True)
                with zipfile.ZipFile(file_path, 'r') as zip_ref:
                    zip_ref.extractall(theme_folder)
                self.load_themes()
        except Exception as e:
            error_message = f"Error importing RRCT: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)

    def test_error_dump(self):
        try:
            raise Exception("This is a test error for dumping.")
        except Exception as e:
            error_message = f"Test error: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)

    def log_error(self, error_message):
        try:
            with open(self.error_log_path, "r") as f:
                error_logs = json.load(f)
        except json.JSONDecodeError:
            error_logs = []

        error_logs.append({"error": error_message, "traceback": traceback.format_exc()})

        with open(self.error_log_path, "w") as f:
            json.dump(error_logs, f, indent=4)

    def load_config(self):
        if not os.path.exists(self.config_path):
            self.config = {}
            self.save_config()  # Create the config file with default values
        else:
            try:
                with open(self.config_path, "r") as f:
                    content = f.read().strip()
                    if content:
                        self.config = json.loads(content)
                    else:
                        self.config = {}
            except json.JSONDecodeError:
                self.config = {}

    def save_config(self):
        with open(self.config_path, "w") as f:
            json.dump(self.config, f, indent=4)

    def load_settings(self):
        if not os.path.exists(self.settings_path):
            self.save_settings()  # Create the settings file with default values
        else:
            try:
                with open(self.settings_path, "r") as f:
                    settings = json.load(f)
                    theme = settings.get("theme", "Default Dark")
                    self.theme_selector.setCurrentText(theme)
                    self.change_theme(self.theme_selector.currentIndex())
                    fullscreen = settings.get("fullscreen", "Windowed")
                    self.fullscreen_selector.setCurrentText(fullscreen)
                    if fullscreen == "Fullscreen":
                        self.showFullScreen()
                    else:
                        self.showNormal()
                    repo_source = settings.get("repo_source", "GitHub")
                    self.repo_selector.setCurrentText(repo_source)
                    github_actions_enabled = settings.get("github_actions_enabled", False)
                    self.github_actions_toggle.setChecked(github_actions_enabled)
            except json.JSONDecodeError:
                self.save_settings()

    def save_settings(self):
        settings = {
            "theme": self.theme_selector.currentText(),
            "fullscreen": self.fullscreen_selector.currentText(),
            "repo_source": self.repo_selector.currentText(),
            "github_actions_enabled": self.github_actions_toggle.isChecked()
        }
        with open(self.settings_path, "w") as f:
            json.dump(settings, f, indent=4)

    def add_cloud_save_location(self, app_name):
        file_dialog = QFileDialog(self)
        file_dialog.setFileMode(QFileDialog.FileMode.Directory)
        if file_dialog.exec():
            selected_folder = file_dialog.selectedFiles()[0]
            self.config[app_name]['cloud_save_location'] = selected_folder
            self.save_config()
            self.sync_cloud_save(app_name)

    def sync_cloud_save(self, app_name):
        cloud_save_location = self.config.get(app_name, {}).get('cloud_save_location')
        if cloud_save_location:
            save_folder = os.path.join("saves", app_name)
            os.makedirs(save_folder, exist_ok=True)
            for item in os.listdir(cloud_save_location):
                s = os.path.join(cloud_save_location, item)
                d = os.path.join(save_folder, item)
                if os.path.isdir(s):
                    shutil.copytree(s, d, dirs_exist_ok=True)
                else:
                    shutil.copy2(s, d)

    def show_context_menu(self, pos, app_name, button):
        menu = QMenu(self)

        change_artwork_action = QAction("Change Artwork", self)
        change_artwork_action.triggered.connect(lambda: self.change_artwork(app_name, button))
        menu.addAction(change_artwork_action)

        cloud_save_action = QAction("Add Cloud Save Location", self)
        if 'cloud_save_location' in self.config.get(app_name, {}):
            cloud_save_action.setText("Change Cloud Save Location")
        cloud_save_action.triggered.connect(lambda: self.add_cloud_save_location(app_name))
        menu.addAction(cloud_save_action)

        delete_action = QAction("Delete", self)
        delete_action.triggered.connect(lambda: self.delete_application(app_name))
        menu.addAction(delete_action)

        menu.exec(button.mapToGlobal(pos))

    def delete_application(self, app_name):
        app_folder = os.path.join("applications", app_name)
        if os.path.exists(app_folder):
            shutil.rmtree(app_folder)
        if app_name in self.config:
            del self.config[app_name]
            self.save_config()
        self.update_library_page()

    def init_gamepads(self):
        for i in range(pygame.joystick.get_count()):
            joystick = pygame.joystick.Joystick(i)
            joystick.init()
            self.gamepads.append(joystick)

    def poll_gamepads(self):
        for gamepad in self.gamepads:
            for event in pygame.event.get():
                if event.type == pygame.JOYBUTTONDOWN:
                    if event.button == 0:  # A button
                        self.activate_focused_widget()
                    elif event.button == 1:  # B button
                        self.show_search_page()
                    elif event.button == 6:  # Left menu button
                        self.show_context_menu(self.current_focus.mapToGlobal(QPoint(0, 0)), self.current_focus.text(), self.current_focus)
                elif event.type == pygame.JOYHATMOTION:
                    if event.value == (0, 1):  # Up
                        self.navigate_focus("up")
                    elif event.value == (0, -1):  # Down
                        self.navigate_focus("down")
                    elif event.value == (-1, 0):  # Left
                        self.navigate_focus("left")
                    elif event.value == (1, 0):  # Right
                        self.navigate_focus("right")

    def navigate_focus(self, direction):
        if not self.current_focus:
            self.current_focus = self.side_panel.itemAt(0).widget()
            if self.current_focus:
                self.current_focus.setFocus()
            return

        if direction == "up":
            next_focus = self.get_next_focus("up")
        elif direction == "down":
            next_focus = self.get_next_focus("down")
        elif direction == "left":
            next_focus = self.get_next_focus("left")
        elif direction == "right":
            next_focus = self.get_next_focus("right")

        if next_focus:
            self.current_focus = next_focus
            self.current_focus.setFocus()

    def get_next_focus(self, direction):
        if direction in ["up", "down"]:
            parent_layout = self.current_focus.parent().layout()
            current_index = parent_layout.indexOf(self.current_focus)
            if direction == "up":
                next_index = (current_index - 1) % parent_layout.count()
            else:
                next_index = (current_index + 1) % parent_layout.count()
            return parent_layout.itemAt(next_index).widget()
        elif direction in ["left", "right"]:
            if self.current_focus.parent() == self.side_panel_widget:
                return self.main_content.currentWidget().layout().itemAt(0).widget()
            else:
                return self.side_panel.itemAt(0).widget()

    def activate_focused_widget(self):
        if self.current_focus:
            if isinstance(self.current_focus, QLineEdit):
                self.current_focus.setFocus()
                # Simulate pressing the Enter key to activate the QLineEdit
                event = QKeyEvent(QEvent.Type.KeyPress, Qt.Key.Key_Return, Qt.KeyboardModifier.NoModifier)
                QApplication.sendEvent(self.current_focus, event)
                event = QKeyEvent(QEvent.Type.KeyRelease, Qt.Key.Key_Return, Qt.KeyboardModifier.NoModifier)
                QApplication.sendEvent(self.current_focus, event)
            elif isinstance(self.current_focus, QComboBox):
                self.current_focus.showPopup()
            elif isinstance(self.current_focus, QPushButton):
                self.current_focus.click()
            elif isinstance(self.current_focus, QScrollArea):
                # Handle QScrollArea differently if needed
                pass

    def eventFilter(self, obj, event):
        if event.type() == QEvent.Type.KeyPress:
            if event.key() in [Qt.Key.Key_Up, Qt.Key.Key_Down, Qt.Key.Key_Left, Qt.Key.Key_Right]:
                direction = {
                    Qt.Key.Key_Up: "up",
                    Qt.Key.Key_Down: "down",
                    Qt.Key.Key_Left: "left",
                    Qt.Key.Key_Right: "right"
                }[event.key()]
                self.navigate_focus(direction)
                return True
            elif event.key() == Qt.Key.Key_Return:
                self.activate_focused_widget()
                return True
        return super().eventFilter(obj, event)

    def load_themes(self):
        try:
            self.theme_selector.clear()
            self.theme_selector.addItem("Default Dark")
            for theme_folder in os.listdir(self.themes_path):
                theme_folder_path = os.path.join(self.themes_path, theme_folder)
                if os.path.isdir(theme_folder_path):
                    theme_file = os.path.join(theme_folder_path, "theme.yaml")
                    if os.path.exists(theme_file):
                        self.theme_selector.addItem(theme_folder)
        except Exception as e:
            error_message = f"Error loading themes: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)

    def change_theme(self, index):
        try:
            selected_theme = self.theme_selector.currentText()
            if selected_theme == "Default Dark":
                self.setStyleSheet(qdarkstyle.load_stylesheet(qt_api='pyqt6'))
            else:
                theme_path = os.path.join(self.themes_path, selected_theme, "theme.yaml")
                if os.path.exists(theme_path):
                    with open(theme_path, 'r') as f:
                        theme = yaml.safe_load(f)
                        self.apply_theme(theme)
                else:
                    raise FileNotFoundError(f"Theme file not found: {theme_path}")
            self.save_settings()
        except Exception as e:
            error_message = f"Error changing theme: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)

    def apply_theme(self, theme):
        try:
            stylesheet = ""
            font_family = None
            for prop, value in theme.items():
                if prop == "font-family":
                    font_path = os.path.join(self.themes_path, self.theme_selector.currentText(), value)
                    font_id = QFontDatabase.addApplicationFont(font_path)
                    if font_id == -1:
                        raise Exception(f"Failed to load font: {font_path}")
                    font_family = QFontDatabase.applicationFontFamilies(font_id)[0]
                elif prop == "panel-background":
                    stylesheet += f"QWidget {{ background-color: {value}; }}\n"
                elif prop == "main-background":
                    stylesheet += f"QMainWindow {{ background-color: {value}; }}\n"
                elif prop == "text-color":
                    stylesheet += f"QLabel, QLineEdit, QPushButton, QComboBox, QScrollArea, QProgressBar, QMenu, QListWidget, QCheckBox, QDialog, QStackedWidget, QSplitter, QMessageBox {{ color: {value}; }}\n"
                elif prop == "button-color":
                    stylesheet += f"QPushButton {{ background-color: {value}; }}\n"
                elif prop == "button-hover-color":
                    stylesheet += f"QPushButton:hover {{ background-color: {value}; }}\n"
                else:
                    stylesheet += f"* {{ {prop}: {value}; }}\n"
            
            if font_family:
                stylesheet = f"* {{ font-family: '{font_family}'; }}\n" + stylesheet
            
            self.setStyleSheet(stylesheet)
        except Exception as e:
            error_message = f"Error applying theme: {e}\n{traceback.format_exc()}"
            self.log_error(error_message)

    def load_plugins(self):
        for folder in os.listdir(self.plugins_path):
            plugin_folder = os.path.join(self.plugins_path, folder)
            if os.path.isdir(plugin_folder):
                base_file = os.path.join(plugin_folder, "base.py")
                if os.path.exists(base_file):
                    try:
                        spec = importlib.util.spec_from_file_location(f"{folder}.base", base_file)
                        module = importlib.util.module_from_spec(spec)
                        spec.loader.exec_module(module)
                        if hasattr(module, "init_plugin"):
                            module.init_plugin(self)
                    except Exception as e:
                        self.log_error(f"Failed loading plugin {folder}: {e}\n{traceback.format_exc()}")

if __name__ == "__main__":
    app = QApplication(sys.argv)
    app.setWindowIcon(QIcon("img/logo.png"))  # Set the application icon for the taskbar
    launcher = RepoRocket()
    launcher.show()
    sys.exit(app.exec())
